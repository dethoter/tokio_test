#![cfg_attr(feature="clippy", feature(plugin))]
#![cfg_attr(feature="clippy", plugin(clippy))]
#![deny(deprecated)]
extern crate tokio_proto;
extern crate tokio_io;
extern crate tokio_core;
extern crate tokio_service;
extern crate tokio_timer;
extern crate futures;
extern crate futures_cpupool;
extern crate bytes;
extern crate bincode;
extern crate rand;
extern crate structopt;
#[macro_use]
extern crate structopt_derive;
extern crate serde;
#[macro_use]
extern crate serde_derive;
mod protocol;

use std::io;
use std::net::SocketAddr;

use futures::{Future, Stream};
use futures::stream::futures_unordered;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::{Framed, Encoder, Decoder};
use tokio_core::reactor::Core;
use tokio_proto::TcpClient;
use tokio_proto::multiplex::{RequestId, ClientProto};
use tokio_service::Service;
use structopt::StructOpt;
use rand::{thread_rng, Rng};
use bytes::{BytesMut, Buf, BufMut, BigEndian};
use protocol::{CountRequest, CountResponse};

#[derive(StructOpt, Debug)]
#[structopt(name = "cli", about = "Console client for counter server")]
struct Args {
    #[structopt(short = "a", long = "address", help = "Server address",
                default_value = "127.0.0.1:5234")]
    address: String,

    #[structopt(short = "r", long = "range_max",
                help = "Max value for randomization range used for requests",
                default_value = "9999999")]
    range_max: usize,

    #[structopt(short = "n", long = "requests", help = "Number of sending requests",
                default_value = "5")]
    requests: usize,
}

struct ClientCodec;
impl Decoder for ClientCodec {
    type Item = (RequestId, CountResponse);
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

impl Encoder for ClientCodec {
    type Item = (RequestId, CountRequest);
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

struct ClientProtocol;
impl<T: AsyncRead + AsyncWrite + 'static> ClientProto<T> for ClientProtocol {
    type Request = CountRequest;
    type Response = CountResponse;
    type Transport = Framed<T, ClientCodec>;
    type BindTransport = Result<Self::Transport, io::Error>;
    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(ClientCodec))
    }
}

fn request_count(address: SocketAddr, range_max: usize, requests: usize) {
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let promises = (0..requests)
        .map(move |_| {
            TcpClient::new(ClientProtocol)
                .connect(&address, &handle)
                .and_then(move |client| {
                    let request = thread_rng().gen_range(0, range_max as u64);
                    println!("Request: {:?}", request);
                    client.call(CountRequest(request))
                })
        })
        .collect::<Vec<_>>();
    let mut work = futures_unordered(promises).into_future();
    loop {
        match core.run(work) {
            Ok((None, _)) => {
                break;
            }
            Ok((Some(response), next_requests)) => {
                println!("Response: {:?}", response);
                work = next_requests.into_future();
            }
            Err((e, next_requests)) => {
                println!("Error: {:?}", e);
                work = next_requests.into_future();
            }
        }
    }
}

fn main() {
    let args = Args::from_args();

    let address = args.address.parse().unwrap();
    println!(
        "Address: {:?}\nRange: [0..{:?}]\nRequests: {:?}",
        address,
        args.range_max,
        args.requests
    );

    request_count(address, args.range_max, args.requests);
}
