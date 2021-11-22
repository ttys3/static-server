# static-server
simple static file server written in Rust based on [axum](https://github.com/tokio-rs/axum) framework

I'm learning Rust and axum.

My thought is simple.

axum has a [static-file-server](https://github.com/tokio-rs/axum/tree/main/examples/static-file-server) example, which only serve static files under a directory and does not list the directory index.

it also has a [templates](https://github.com/tokio-rs/axum/tree/main/examples/templates) example which uses askama as template engine to parse a Jinja2 like template.

I thought I could simply combine the code of the two and my job is done -_-

But things didn't go in the way I thought


## why created this ?

long time ago, I used to start a static http server like this (using the python3 built-in http server) :

```shell
python3 -m http.server -d .
```
since I'm learning Rust, and I found an interesting web framework that is `Axum`, I want to use Axum to implement a simple static server mainly for studying purposes.

the python one support some command line flags:

```shell
❯ python3 -m http.server -h
usage: server.py [-h] [--cgi] [--bind ADDRESS] [--directory DIRECTORY] [port]

positional arguments:
  port                  Specify alternate port [default: 8000]

optional arguments:
  -h, --help            show this help message and exit
  --cgi                 Run as CGI Server
  --bind ADDRESS, -b ADDRESS
                        Specify alternate bind address [default: all interfaces]
  --directory DIRECTORY, -d DIRECTORY
                        Specify alternative directory [default:current directory]
```

so does this one:

```shell
❯ static-server -h
static-server 0.4.2
A simple static file server written in Rust based on axum framework.

USAGE:
    static-server [OPTIONS]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -a, --addr <addr>        set the listen addr [default: 127.0.0.1]
    -l, --log <log-level>    set the log level [default: debug]
    -p, --port <port>        set the listen port [default: 3000]
    -r, --root <root-dir>    set the root directory [default: .]
```
