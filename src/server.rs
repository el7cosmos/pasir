// #[tokio::main]
// pub(crate) async fn server(
//   config: &Config,
// ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//   // We create a TcpListener and bind it to 127.0.0.1:3000
//   let listener = TcpListener::bind((config.address(), config.port())).await?;
//
//   let config = config.clone();
//
//   // We start a loop to continuously accept incoming connections
//   loop {
//     let (stream, sock) = listener.accept().await?;
//     let local_a = stream.local_addr().unwrap();
//     let local_a = stream.local_addr().unwrap();
//     let peer_a = stream.peer_addr().unwrap();
//
//     let config = config.clone();
//
//     // Use an adapter to access something implementing `tokio::io` traits as if they implement
//     // `hyper::rt` IO traits.
//     let io = TokioIo::new(stream);
//
//     // Spawn a tokio task to serve multiple connections concurrently
//     tokio::task::spawn(async move {
//       // Finally, we bind the incoming connection to our `hello` service
//       if let Err(err) = http1::Builder::new()
//         // `service_fn` converts our function in a `Service`
//         .serve_connection(
//           io,
//           service_fn(
//             async |request: Request<Incoming>| -> Result<Response<Full<Bytes>>, Infallible> {
//               dbg!(local_a);
//               let sapi = Sapi::new(Sapi::builder());
//               Service::new(&config, sapi, peer_a).call(request).await
//             },
//           ),
//         )
//         .await
//       {
//         eprintln!("Error serving connection: {:?}", err);
//       }
//     });
//   }
// }
