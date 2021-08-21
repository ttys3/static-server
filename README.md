# static-server
simple static file server written in Rust based on [axum](https://github.com/tokio-rs/axum) framework

I'm learning Rust and axum.

My thought is simple. 

axum has a [static-file-server](https://github.com/tokio-rs/axum/tree/main/examples/static-file-server) example, which only serve static files under a directory and does not list the directory index.

it also has a [templates](https://github.com/tokio-rs/axum/tree/main/examples/templates) example which uses askama as template engine to parse a Jinja2 like template.

I thought I could simply combine the code of the two and my job is done -_- 

But things didn't go in the way I thought
