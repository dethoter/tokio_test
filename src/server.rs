#![cfg_attr(feature="clippy", feature(plugin))]
#![cfg_attr(feature="clippy", plugin(clippy))]
#![deny(deprecated)]
extern crate tokio_proto;
extern crate tokio_io;
extern crate tokio_service;
extern crate tokio_timer;
extern crate futures;
extern crate futures_cpupool;
extern crate bytes;
extern crate bincode;
extern crate structopt;
#[macro_use]
extern crate structopt_derive;
extern crate serde;
#[macro_use]
extern crate serde_derive;
mod protocol;

use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use futures::{future, Future};
use futures_cpupool::CpuPool;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::{Framed, Encoder, Decoder};
use tokio_timer::Timer;
use tokio_proto::TcpServer;
use tokio_proto::multiplex::{RequestId, ServerProto};
use tokio_service::Service;
use structopt::StructOpt;
use bytes::{BytesMut, Buf, BufMut, BigEndian};
use protocol::{CountRequest, CountResponse};

#[derive(StructOpt, Debug)]
#[structopt(name = "serv", about = "Server that counts")]
struct Args {
    #[structopt(short = "p", long = "port", help = "Server's port", default_value = "5234")]
    port: u16,

    #[structopt(short = "t", long = "timeout", help = "Timeout per 1 task", default_value = "5")]
    timeout: usize,

    #[structopt(short = "j", long = "threads", help = "Number of threads", default_value = "0")]
    threads: usize,
}

struct ServerCodec;
impl Decoder for ServerCodec {
    type Item = (RequestId, CountRequest);
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, io::Error> {
        if buf.len() >= 4 {
            let length = io::Cursor::new(&buf.split_to(4)).get_u32::<BigEndian>() as usize;
            if buf.len() >= length {
                return Ok(bincode::deserialize(&buf.split_to(length)).ok());
            }
        }

        Ok(None)
    }
}

impl Encoder for ServerCodec {
    type Item = (RequestId, CountResponse);
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item, buf: &mut BytesMut) -> io::Result<()> {
        let bytes = bincode::serialize(&item, bincode::Infinite).map_err(|_| {
            io::ErrorKind::InvalidData
        })?;
        let length = bytes.len();
        buf.reserve(4 + length);
        buf.put_u32::<BigEndian>(length as u32);
        buf.put_slice(&bytes);
        Ok(())
    }
}

struct ServerProtocol;
impl<T: AsyncRead + AsyncWrite + 'static> ServerProto<T> for ServerProtocol {
    type Request = CountRequest;
    type Response = CountResponse;
    type Transport = Framed<T, ServerCodec>;
    type BindTransport = Result<Self::Transport, io::Error>;
    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(ServerCodec))
    }
}

struct Counter {
    thread_pool: CpuPool,
    timeout: Duration,
}

impl Service for Counter {
    type Request = CountRequest;
    type Response = CountResponse;
    type Error = io::Error;
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let CountRequest(num) = req;
        println!("Request: {:?}", num);

        let timeout = Timer::default().sleep(self.timeout).then(|_| Err(()));
        let task = self.thread_pool
            .spawn_fn(move || Ok(count(num)))
            .select(timeout)
            .map(|(r, _)| r);

        // FIXME: Timeout doesn't have and_then() implementation
        // let task = Timer::default().timeout(task, self.timeout);

        Box::new(
            task.and_then(move |d| {
                println!("Response: Duration({:?},{:?})", num, &d);
                future::ok(CountResponse::Duration(num, d))
            }).or_else(move |_| {
                    println!("Response: Timeout({:?})", num);
                    future::ok(CountResponse::Timeout(num))
                }),
        )
    }
}

fn count(max: u64) -> Duration {
    let now = Instant::now();

    let mut i = 0;
    while i < max {
        i += 1;
    }

    now.elapsed()
}

fn serve(address: SocketAddr, pool: CpuPool, duration: Duration) {
    TcpServer::new(ServerProtocol, address).serve(move || {
        Ok(Counter {
            thread_pool: pool.clone(),
            timeout: duration,
        })
    });
}

fn main() {
    let args = Args::from_args();

    let pool = if args.threads == 0 {
        CpuPool::new_num_cpus()
    } else {
        CpuPool::new(args.threads)
    };

    let duration = Duration::from_secs(args.timeout as u64);
    let address = format!("0.0.0.0:{}", args.port).parse().unwrap();
    println!(
        "Address: {:?}\nTimeout: {:?}\nThreads: {:?}",
        &address,
        args.timeout,
        if args.threads != 0 {
            args.threads.to_string()
        } else {
            "native".into()
        }
    );

    serve(address, pool, duration);
}
