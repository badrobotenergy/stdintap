use std::{
    io::{ErrorKind, Read},
    pin::Pin,
    time::{Duration, Instant},
};

use bytes::{Bytes, BytesMut};
use clap::Parser;
use std::fmt::Write;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::broadcast::error::RecvError,
};

/// Accept lines from stdin and allow socket clients to tap into them
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[clap(flatten)]
    listener: tokio_listener::ListenerAddressPositional,

    /// Size of broadcast channel for serving the lines
    #[clap(long, short = 'q', default_value = "16")]
    qlen: usize,

    /// Slow down reading from stdin if connected clients are slow in reading output
    #[clap(long)]
    backpressure: bool,

    /// Inject special lines that denote missed content due to slow reading
    /// In `--backpressure` mode, it will insert announcements that backpressure is applied
    /// Additionally, stdin EOFs will also be announced.
    ///
    /// When `--timestamps` are active, special lines are separated from the timestamp by
    /// spaces instead of tabs.
    ///
    /// Note that overrun announcements may exacerbate overruns.
    #[clap(long, short = 'x')]
    announce_overruns: bool,

    /// Disconnect clients when they are too slow to read lines
    #[clap(long)]
    disconnect_on_overruns: bool,

    /// Prefix messages with a monotone timestamps
    #[clap(long, short = 't')]
    timestamps: bool,

    /// Inject initial message at the beginning of each client connection
    #[clap(long, short = 'H')]
    hello_message: bool,

    /// Automatically split lines longer than this
    #[clap(long, default_value = "65536")]
    max_line_size: usize,

    /// Separata lines by zero byte instead of \n
    #[clap(long)]
    zero_separated: bool,

    /// Also copy stdin to stdout
    #[clap(long, short='T')]
    tee: bool,
}

#[derive(Clone)]
enum MsgInner {
    Content(Bytes),
    Eof,
    Backpressure,
}

#[derive(Clone)]
struct Msg {
    ts: Instant,
    inner: MsgInner,
}

struct TimestampPrinter {
    begin: Instant,
    buf: String,
}

impl TimestampPrinter {
    fn new(begin: Instant) -> Self {
        Self {
            begin,
            buf: String::with_capacity(6 + 1 + 6 + 1),
        }
    }

    async fn print(
        &mut self,
        mut conn: Pin<&mut impl AsyncWrite>,
        ts: Instant,
        sep: char,
    ) -> std::io::Result<()> {
        let x = ts - self.begin;
        let s = x.as_secs();
        let m = x.subsec_millis();
        self.buf.clear();
        let _ = write!(self.buf, "{s:06}.{m:06}{sep}");
        conn.write_all(self.buf.as_bytes()).await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let Args {
        listener,
        qlen,
        backpressure,
        announce_overruns,
        disconnect_on_overruns,
        timestamps,
        hello_message,
        max_line_size,
        zero_separated,
        tee,
    } = Args::parse();

    if qlen < 2 && backpressure {
        anyhow::bail!("backpressure requires qlen at least 2");
    }

    let tx = tokio::sync::broadcast::Sender::<Msg>::new(qlen);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let tx2 = tx.clone();

    let begin = Instant::now();
    let byte_to_look_at = if zero_separated { b'\0' } else { b'\n' };
    let separator_char = if zero_separated { '\0' } else { '\n' };
    std::thread::spawn(move || {
        let _shutdown_tx = shutdown_tx;
        let si = std::io::stdin();
        let mut si = si.lock();
        let tx = tx2;

        let so_;
        let mut so = if tee {
            so_ = std::io::stdout();
            Some(so_.lock())
        } else {
            None
        };

        let mut buf = BytesMut::with_capacity(8192*2);

        let mut noticed_about_nonblocking_stdin = false;
        let mut debt = 0usize;
        loop {
            buf.reserve((8192 + debt).saturating_sub(buf.capacity()));
            buf.resize(buf.capacity(), 0);

            let n = match si.read(&mut buf[debt..]) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::Interrupted => {
                    dbg!();
                    continue;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    dbg!();
                    if !noticed_about_nonblocking_stdin {
                        eprintln!(
                            "Warning: stdin is set to nonblocking mode. Using a timer to poll it."
                        );
                        noticed_about_nonblocking_stdin = true;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(e) => {
                    eprintln!("Reading from stdio: {e}");
                    break;
                }
            };
            if let Some(ref mut so) = so {
                if std::io::Write::write_all(so, &buf[debt..(debt+n)]).is_err() {
                    eprintln!("Writing to stdout failed");
                    break;
                }
            }
            let mut n = n;

            assert!(buf.len() >= debt + n);
            'restarter: loop {
                for i in 0..n {
                    if buf[debt + i] == byte_to_look_at || debt + i == max_line_size {
                        let content = buf.split_to(debt + i + 1).freeze();
                        debt = 0;
                        n -= i + 1;

                        let ts = Instant::now();

                        if !backpressure || tx.len() < qlen - 1 {
                            let _ = tx.send(Msg {
                                ts,
                                inner: MsgInner::Content(content),
                            });
                        } else {
                            let _ = tx.send(Msg {
                                ts,
                                inner: MsgInner::Backpressure,
                            });
                            let mut wait_micros = 1;
                            while tx.len() >= qlen - 1 {
                                std::thread::sleep(Duration::from_micros(wait_micros));
                                if wait_micros < 65536 {
                                    wait_micros *= 2;
                                }
                            }
                            let _ = tx.send(Msg {
                                ts,
                                inner: MsgInner::Content(content),
                            });
                        }

                        continue 'restarter;
                    }
                }
                break 'restarter;
            }

            debt += n;
        }

        let _ = tx.send(Msg {
            ts: Instant::now(),
            inner: MsgInner::Eof,
        });
    });

    let mut listener = listener.bind().await?;

    loop {
        let ret = tokio::select! {
            _ = &mut shutdown_rx => break,
            x = listener.accept() => x,
        };
        let Ok((conn, _addr)) = ret else {
            eprintln!("Error accepting socket");
            break;
        };
        let mut rx = tx.subscribe();

        tokio::task::spawn(async move {
            let ret: anyhow::Result<()> = async move {
                let conn = tokio::io::BufWriter::new(conn);
                tokio::pin!(conn);
                let mut tsprinter = TimestampPrinter::new(begin);

                let mut overrun_counter = 0;

                if hello_message {
                    if timestamps {
                        tsprinter.print(conn.as_mut(), Instant::now(), ' ').await?;
                    }
                    let mut buf = String::with_capacity(16);
                    let _ = write!(buf, "HELLO{separator_char}");
                    conn.as_mut().write_all(buf.as_bytes()).await?;
                    conn.as_mut().flush().await?;
                }

                loop {
                    match rx.recv().await {
                        Ok(msg) => {
                            match msg.inner {
                                MsgInner::Content(b) => {
                                    if announce_overruns && overrun_counter > 0 {
                                        if timestamps {
                                            tsprinter
                                                .print(conn.as_mut(), Instant::now(), ' ')
                                                .await?;
                                        }
                                        let mut buf = String::with_capacity(16);
                                        let _ = write!(
                                            buf,
                                            "OVERRUN {overrun_counter}{separator_char}"
                                        );
                                        conn.as_mut().write_all(buf.as_bytes()).await?;
                                        overrun_counter = 0;
                                    }
                                    if timestamps {
                                        tsprinter.print(conn.as_mut(), msg.ts, '\t').await?;
                                    }
                                    conn.as_mut().write_all(&b).await?;
                                }
                                MsgInner::Eof => break,
                                MsgInner::Backpressure => {
                                    if announce_overruns {
                                        if timestamps {
                                            tsprinter.print(conn.as_mut(), msg.ts, ' ').await?;
                                        }

                                        let mut buf = String::with_capacity(16);
                                        let _ = write!(buf, "BACKPRESSURE{separator_char}");
                                        conn.as_mut().write_all(buf.as_bytes()).await?;
                                    }
                                }
                            }
                            if rx.len() == 0 {
                                conn.as_mut().flush().await?;
                            }
                        }
                        Err(e) => match e {
                            RecvError::Closed => break,
                            RecvError::Lagged(n) => {
                                overrun_counter += n;
                                if disconnect_on_overruns {
                                    return Ok(());
                                }
                            }
                        },
                    }
                }
                if announce_overruns {
                    if timestamps {
                        tsprinter.print(conn.as_mut(), Instant::now(), ' ').await?;
                    }
                    let mut buf = String::with_capacity(16);
                    let _ = write!(buf, "EOF{separator_char}");
                    conn.as_mut().write_all(buf.as_bytes()).await?;
                    conn.as_mut().flush().await?;
                }

                Ok(())
            }
            .await;
            let _ = ret;
        });
    }
    let mut patience_points = 10;
    while tx.receiver_count() > 0 {
        patience_points -= 1;
        if patience_points == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Ok(())
}
