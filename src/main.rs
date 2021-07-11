use std::panic::{set_hook, PanicInfo};
use tokio::runtime::Builder;
use tokio::net::TcpListener;

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

fn main() {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
        linstener_work(&listener).await;
    });
}
