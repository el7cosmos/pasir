use bytes::Buf;
use futures_util::FutureExt;
use hyper::body::{Body, Frame};
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc::error::SendError;
use tokio::sync::oneshot::Receiver;
use tokio::sync::{mpsc, oneshot};

pub struct UnboundChannel<D> {
  rx_frame: mpsc::UnboundedReceiver<Frame<D>>,
  rx_finish: Receiver<()>,
}

impl<D> UnboundChannel<D> {
  pub fn new() -> (Sender<D>, Self) {
    let (tx_frame, rx_frame) = mpsc::unbounded_channel();
    let (tx_finish, rx_finish) = oneshot::channel();
    (Sender { tx_frame, tx_finish }, Self { rx_frame, rx_finish })
  }
}

impl<D> Body for UnboundChannel<D>
where
  D: Buf,
{
  type Data = D;
  type Error = Infallible;

  fn poll_frame(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
    match self.rx_frame.poll_recv(cx) {
      Poll::Ready(frame @ Some(_)) => return Poll::Ready(frame.map(Ok)),
      Poll::Ready(None) | Poll::Pending => {}
    }

    match self.rx_finish.poll_unpin(cx) {
      Poll::Ready(_) => return Poll::Ready(None),
      Poll::Pending => {}
    }

    Poll::Pending
  }
}

#[derive(Debug)]
pub struct Sender<D> {
  tx_frame: mpsc::UnboundedSender<Frame<D>>,
  tx_finish: oneshot::Sender<()>,
}

impl<D> Sender<D> {
  pub fn send(&mut self, frame: Frame<D>) -> Result<(), SendError<Frame<D>>> {
    self.tx_frame.send(frame)
  }

  /// Aborts the body in an abnormal fashion.
  pub fn abort(self) {
    self.tx_finish.send(()).ok();
  }
}

#[cfg(test)]
mod tests {
  use crate::unbound_channel::UnboundChannel;
  use bytes::Bytes;
  use http_body_util::BodyExt;
  use hyper::body::Frame;

  #[tokio::test]
  async fn empty() {
    let (tx, body) = UnboundChannel::<Bytes>::new();
    drop(tx);

    let collected = body.collect().await.unwrap();
    assert!(collected.to_bytes().is_empty());
  }

  #[tokio::test]
  async fn can_send_data() {
    let (mut tx, body) = UnboundChannel::<Bytes>::new();

    tokio::spawn(async move {
      assert!((tx.send(Frame::data(Bytes::from("Hel")))).is_ok());
      assert!(tx.send(Frame::data(Bytes::from("lo!"))).is_ok());
    });

    let collected = body.collect().await.unwrap();
    assert_eq!(collected.to_bytes(), "Hello!");
  }

  #[tokio::test]
  async fn abort_will_close() {
    let (tx, body) = UnboundChannel::<Bytes>::new();

    tokio::spawn(async move {
      tx.abort();
    });

    assert!(body.collect().await.is_ok());
  }
}
