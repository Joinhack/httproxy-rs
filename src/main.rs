use std::panic::{set_hook, PanicInfo};
use tokio::net::TcpListener;
use tokio::runtime::Builder;

use getopts::Options;
use std::env;

mod conn;
use conn::*;

fn panic_hook(info: &PanicInfo<'_>) {
    if let Some(s) = info.payload().downcast_ref::<String>() {
        if let Some(loc) = info.location() {
            eprintln!("file:{}:{} {}", loc.file(), loc.line(), s);
        } else {
            eprintln!("{}", s);
        }
    } else {
        eprintln!("{:?}", info);
    }
}

async fn linstener_work(listener: &TcpListener) {
    loop {
        let (tcp_stream, addr) = listener.accept().await.unwrap();
        tokio::spawn(async move {
            let mut conn = Connect::new(tcp_stream);
            set_hook(Box::new(panic_hook));
            conn.process_remote(addr).await;
        });
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

    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind(&addr).await.unwrap();
        linstener_work(&listener).await;
    });
}
