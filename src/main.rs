use tokio::net::TcpListener;
use tokio::runtime::Builder;

use getopts::Options;
use std::env;
use std::sync::Arc;

mod conn;
use conn::*;

#[derive(Clone)]
struct Server {
    inner: Arc<ServerInner>,
}

struct ServerInner {
    listener_addr: String,
    username: Option<String>,
    password: Option<String>,
    listener: TcpListener,
}

impl Server {
    fn new(
        listener_addr: String,
        username: Option<String>,
        password: Option<String>,
        listener: TcpListener,
    ) -> Server {
        Server {
            inner: Arc::new(ServerInner {
                listener_addr,
                listener,
                username,
                password,
            }),
        }
    }

    fn listener_addr_ref(&self) -> &str {
        &self.inner.listener_addr
    }

    fn listener_ref(&self) -> &TcpListener {
        &self.inner.listener
    }

    fn get_username_ref(&self) -> Option<&str> {
        match self.inner.username {
            Some(ref s) => Some(s.as_str()),
            None => None,
        }
    }

    fn get_password_ref(&self) -> Option<&str> {
        match self.inner.password {
            Some(ref s) => Some(s.as_str()),
            None => None,
        }
    }

    async fn listener_work(&self) {
        let listener = self.listener_ref();
        loop {
            let (tcp_stream, addr) = listener.accept().await.unwrap();
            let server = self.clone();
            tokio::spawn(async move {
                let mut conn = Connect::new(tcp_stream, server);
                conn.process_remote(addr).await;
            });
        }
    }
}

fn print_usage(prg: &str, opts: &Options) {
    let brief = format!("Usage: {} FILE [options]", prg);
    print!("{}", opts.usage(&brief));
}

fn main() {
    let args = env::args().collect::<Vec<String>>();
    let mut opts = Options::new();
    opts.optopt(
        "l",
        "",
        "set listener address default:0.0.0.0:8080",
        "Listener",
    );
    opts.optopt("p", "", "set password", "Password");
    opts.optopt("u", "", "set username", "Username");
    let matcher = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(_) => {
            print_usage(&args[0], &opts);
            return;
        }
    };

    let addr = match matcher.opt_str("l") {
        Some(addr) => addr,
        None => "0.0.0.0:8080".into(),
    };

    let username = matcher.opt_str("u");

    let password = matcher.opt_str("p");

    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let l = TcpListener::bind(&addr).await.unwrap();
        let server = Server::new(addr, username, password, l);
        println!("server listen on {}", server.listener_addr_ref());
        server.listener_work().await;
    });
}
