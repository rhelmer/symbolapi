Introduction
--------

Symbolapi is a Web server for symbolicating Firefox stacks. It matches PC addresses to modules in memory and looks up the corresponding
function names in server-side symbol files (.SYM files).

If you're interested in setting up local symbols for use with the Gecko profiler for Firefox, the following page will be useful to you:
[Profiling with the Built in Profiler and Local Symbols on Windows](https://developer.mozilla.org/en/Performance/Profiling_with_the_Built-in_Profiler_and_Local_Symbols_on_Windows)

This project is inspired by the [Snappy Symbolication Server](https://github.com/mozilla/Snappy-Symbolication-Server/) and is intended
as a drop-in replacement.

Building and Running
--------------------

Symbolapi is written in Rust and uses Cargo. Make sure you have the latest stable version of Rust installed from
[Rust's install page](http://www.rust-lang.org/install.html):

```
  cargo build
```

You can then run the symbolapi server:

```
  cargo run
```

Symbolapi listens on all interfaces and port 8080 by default.


Deploying on Ubuntu using Nginx
-------------------------------
Build a deployable binary:

```
  sudo apt-get install libssl-dev gcc git
  git clone https://github.com/rhelmer/symbolapi.git
  cd symbolapi
  cargo build --release
```

This produces a build in `./target/release/symbolapi` that can be deployed
to your server.

Copy the `symbolapi` binary to your server, and run it under a supervisor
process (systemd, init script, etc - or tmux in a pinch).

On the server, install and configure Nginx:
```
  sudo apt-get install nginx
```

Override the default location in the default site in
/etc/nginx/sites-available/default:
```
       location / {
                proxy_pass   http://127.0.0.1:5000;
        }
```

Now restart nginx:

```
  sudo service nginx restart
```

Deploying to Heroku
-------------------

Create the app endpoint:
```
  # FIXME can go back to using emk's buildpack when
  # https://github.com/emk/heroku-buildpack-rust/pull/7 lands
  heroku create symbolapi --buildpack https://github.com/rhelmer/heroku-buildpack-rust.git
```

Deploy:
```
$ git push heroku master
```
