# stdintap

Read lines from stdin and copy them to all connected clients.

## Features

* UNIX socket support (including Linux's abstract namespace)
* Adjustable behaviour of handling slow clients: lines may be dropped or backpressure can be applied.
* For slow clients, `stdintap` can announce number of omitted lines. Alternatively, it can announce moments where backpressure was applied.
* History mode - `stdintap` can keep N last lines and replay them to connected clients. There is optional announcement where history ends and realtime data begins.
* Optional printing of timestamps (using monotonic timer) and sequence numbers.
* Maximum line size is limited (adjustable). You can use zero byte instead of newline if needed.
* You can request content to be also forwarded to stdout.
* Reasonable performance.

## Examples

Simple

```
| $ stdintap 127.0.0.1:1234
| A
| B
| C    | $ nc 127.0.0.1 1234
| D    | D
| E    | E
| F    | F    | $ nc 127.0.0.1 1234
| G    | G    | G
| H    | ^C   | H
| I    | $    | I
| J           | J
...
```

Advanced options

```
| $ stdintap @qqq -x -H --history 2 -t --seqn
| a
| b
| c
| d
| e   
|     | $ socat - abstract:qqq
|     | 000004.000452   3       d
|     | 000006.000780   4       e
|     | 000012.000967 HELLO
| f   | 000085.000356   5       f
| g   | 000090.000084   6       g
| ^D  | 000134.000408 EOF
| $   | $
```

<details><summary> Overrun announcements </summary>

```
| $ stdintap @qqq -x --qlen 1 --send-buffer-size 16
|                   | $ socat - abstract:qqq,rcvbuf=16
| 000000000000000   | 000000000000000
| 111111111111111   | 111111111111111
| 222222222222222   | 222222222222222
|                   | ^Z
|                   | [1]+  Stopped
|                   | $
| 333333333333333
| 444444444444444
| 555555555555555
| 666666666666666
| 777777777777777
| 888888888888888
| 999999999999999
| AAAAAAAAAAAAAAA
| BBBBBBBBBBBBBBB
| CCCCCCCCCCCCCCC
| DDDDDDDDDDDDDDD
| EEEEEEEEEEEEEEE
| FFFFFFFFFFFFFFF
| GGGGGGGGGGGGGGG
|                   | $ fg
|                   | 333333333333333
|                   | 444444444444444
|                   | 555555555555555
|                   | 666666666666666
|                   | 777777777777777
|                   | 888888888888888
|                   | 999999999999999
|                   | AAAAAAAAAAAAAAA
|                   | OVERRUN 5
|                   | GGGGGGGGGGGGGGG
| HHHHHHHHHHHHHHH   | HHHHHHHHHHHHHHH
| ^D                | EOF
| $                 | $
```

</details>

## Installation

Download a pre-built executable from [Github releases](https://github.com/vi/stdintap/releases) or install from source code with `cargo install --path .`  or `cargo install stdintap`.

## CLI options

<details><summary> stdintap --help output (not including the `tokio-listener` options)</summary>

```
CLI tool to read lines from stdin and broadcast them to connected TCP clients

Usage: 

Arguments:
  <LISTEN_ADDRESS>
          Socket address to listen for incoming connections.
          
          Various types of addresses are supported:
          
          * TCP socket address and port, like 127.0.0.1:8080 or [::]:80
          
          * UNIX socket path like /tmp/mysock or Linux abstract address like @abstract
          
          * Special keyword "sd-listen" to accept connections from file descriptor 3 (e.g. systemd socket activation). You can also specify a named descriptor after a colon or * to use all passed sockets.
          
          Note that some features may be disabled by compile-time settings.

Options:
  -q, --qlen <QLEN>
          Size of broadcast channel for serving the lines
          
          [default: 16]

      --backpressure
          Slow down reading from stdin if connected clients are slow in reading output

  -x, --announce-overruns
          Inject special lines that denote missed content due to slow reading In `--backpressure` mode, it will insert announcements that backpressure is applied Additionally, stdin EOFs will also be announced.
          
          When `--timestamps` are active, special lines are separated from the timestamp by spaces instead of tabs.
          
          Note that overrun announcements may exacerbate overruns.

      --disconnect-on-overruns
          Disconnect clients when they are too slow to read lines

  -t, --timestamps
          Prefix messages with a monotone timestamps

  -H, --hello-message
          Inject initial message at the beginning of each client connection
          
          With --history option, the hello message appears after the history, before the "online" content.

      --max-line-size <MAX_LINE_SIZE>
          Automatically split lines longer than this
          
          [default: 65536]

  -0, --zero-separated
          Separata lines by zero byte instead of \n

  -T, --tee
          Also copy stdin to stdout

      --seqn
          Print sequence numbers of lines

      --history <HISTORY>
          Remember and this number of lines and replay them to each connecting client

      --require-observer
          Don't read from stdin unless at least one client is connected.
          
          Does not gurantee lack of dropped lines on disconnections.

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

```

(manually removed an inapplicable phrase)

</details>

<details><summary> Socket listening options </summary>

```
      --unix-listen-unlink
          remove UNIX socket prior to binding to it

      --unix-listen-chmod <UNIX_LISTEN_CHMOD>
          change filesystem mode of the newly bound UNIX socket to `owner`, `group` or `everybody`

      --unix-listen-uid <UNIX_LISTEN_UID>
          change owner user of the newly bound UNIX socket to this numeric uid

      --unix-listen-gid <UNIX_LISTEN_GID>
          change owner group of the newly bound UNIX socket to this numeric uid

      --sd-accept-ignore-environment
          ignore environment variables like LISTEN_PID or LISTEN_FDS and unconditionally use file descritor `3` as a socket in sd-listen or sd-listen-unix modes

      --tcp-keepalive <TCP_KEEPALIVE>
          set SO_KEEPALIVE settings for each accepted TCP connection.
          
          Value is a colon-separated triplet of time_ms:count:interval_ms, each of which is optional.

      --tcp-reuse-port
          Try to set SO_REUSEPORT, so that multiple processes can accept connections from the same port in a round-robin fashion

      --recv-buffer-size <RECV_BUFFER_SIZE>
          Set socket's SO_RCVBUF value

      --send-buffer-size <SEND_BUFFER_SIZE>
          Set socket's SO_SNDBUF value

      --tcp-only-v6
          Set socket's IPV6_V6ONLY to true, to avoid receiving IPv4 connections on IPv6 socket

      --tcp-listen-backlog <TCP_LISTEN_BACKLOG>
          Maximum number of pending unaccepted connections
```


</details>

## See also

* https://github.com/vi/line2httppost + https://github.com/vi/postsse
* https://github.com/vi/websocat + https://github.com/vi/wsbroad
