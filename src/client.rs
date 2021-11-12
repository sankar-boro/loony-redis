use std::collections::VecDeque;
use std::{cell::RefCell, fmt, future::Future, pin::Pin, rc::Rc, task::Context, task::Poll};

use loony::util::{poll_fn, Either, Ready };
use loony::{channel::pool, framed::State, service::Service};
use loony::service::Transform;
use loony::web::{ WebRequest, WebResponse, ErrorRenderer };

use super::cmd::Command;
use super::codec::{Codec, Request, Response};
use super::errors::{CommandError, Error};

type Queue = Rc<RefCell<VecDeque<pool::Sender<Result<Response, Error>>>>>;

#[derive(Clone)]
/// Shared redis client
pub struct Client {
    state: State,
    queue: Queue,
    pool: pool::Pool<Result<Response, Error>>,
}

impl Client {
    pub(crate) fn new(state: State) -> Self {
        let queue: Queue = Rc::new(RefCell::new(VecDeque::new()));

        // read redis response task
        let state2 = state.clone();
        let queue2 = queue.clone();
        loony::rt::spawn(async move {
            let read = state2.read();

            poll_fn(|cx| {
                loop {
                    match read.decode(&Codec) {
                        Err(e) => {
                            if let Some(tx) = queue2.borrow_mut().pop_front() {
                                let _ = tx.send(Err(e));
                            }
                            queue2.borrow_mut().clear();
                            state2.shutdown_io();
                            return Poll::Ready(());
                        }
                        Ok(Some(item)) => {
                            if let Some(tx) = queue2.borrow_mut().pop_front() {
                                let _ = tx.send(Ok(item));
                            } else {
                                log::error!("Unexpected redis response: {:?}", item);
                            }
                        }
                        Ok(None) => break,
                    }
                }

                if !state2.is_open() {
                    return Poll::Ready(());
                }
                state2.register_dispatcher(cx.waker());
                Poll::Pending
            })
            .await
        });

        Client {
            state,
            queue,
            pool: pool::new(),
        }
    }

    /// Execute redis command
    pub fn exec<T>(&self, cmd: T) -> impl Future<Output = Result<T::Output, CommandError>>
    where
        T: Command,
    {
        let is_open = self.state.is_open();
        let fut = self.call(cmd.to_request());

        async move {
            if !is_open {
                Err(CommandError::Protocol(Error::Disconnected))
            } else {
                fut.await
                    .map_err(CommandError::Protocol)
                    .and_then(|res| T::to_output(res.into_result().map_err(CommandError::Error)?))
            }
        }
    }

    /// Delete all the keys of the currently selected DB.
    pub async fn flushdb(&self) -> Result<(), Error> {
        self.call("FLUSHDB".into()).await?;
        Ok(())
    }

    /// Returns true if underlying transport is connected to redis
    pub fn is_connected(&self) -> bool {
        self.state.is_open()
    }
}

impl Service for Client {
    type Request = Request;
    type Response = Response;
    type Error = Error;
    type Future = Either<CommandResult, Ready<Response, Error>>;

    fn poll_ready(&self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if !self.state.is_open() {
            Poll::Ready(Err(Error::Disconnected))
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn call(&self, req: Request) -> Self::Future {
        if let Err(e) = self.state.write().encode(req, &Codec) {
            Either::Right(Ready::Err(e))
        } else {
            let (tx, rx) = self.pool.channel();
            self.queue.borrow_mut().push_back(tx);
            Either::Left(CommandResult { rx })
        }
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("connected", &self.state.is_open())
            .finish()
    }
}

impl<S, Err> Transform<S> for Client
where
    S: Service<Request = WebRequest<Err>, Response = WebResponse>,
    S::Future: 'static,
    Err: ErrorRenderer,
    Err::Container: From<S::Error>,
    // CorsError: WebResponseError<Err>,
{
    // type Request = WebRequest<Err>;
    // type Response = WebResponse;
    // type Error = S::Error;
    // type InitError = ();
    // type Transform = CorsMiddleware<S>;
    // type Future = Ready<Result<CorsMiddleware<S>, ()>>;
    type Service = ClientMiddleware<S>;

    fn new_transform(&self, service: S) -> Self::Service { // Self::Future
        ClientMiddleware {
            state: self.state.clone(),
            queue: self.queue.clone(),
            pool: self.pool.clone(),
            service
        }
    }
}



/// `Middleware` for Cross-origin resource sharing support
///
/// The Cors struct contains the settings for CORS requests to be validated and
/// for responses to be generated.
#[derive(Clone)]
pub struct ClientMiddleware<S> {
    state: State,
    queue: Queue,
    pool: pool::Pool<Result<Response, Error>>,
    service: S
}

impl<S, Err> Service for ClientMiddleware<S>
where
    S: Service<Request = WebRequest<Err>, Response = WebResponse>,
    S::Future: 'static,
    Err: ErrorRenderer,
    Err::Container: From<S::Error>,
    // CorsError: WebResponseError<Err>,
{
    type Request = WebRequest<Err>;
    type Response = WebResponse;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn poll_shutdown(&self, cx: &mut Context<'_>, is_error: bool) -> Poll<()> {
        self.service.poll_shutdown(cx, is_error)
    }

    fn call(&self, req: WebRequest<Err>) -> Self::Future {
        self.service.call(req)
    }
}


pub struct CommandResult {
    rx: pool::Receiver<Result<Response, Error>>,
}

impl Future for CommandResult {
    type Output = Result<Response, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Ready(Ok(res)) => Poll::Ready(res),
            Poll::Ready(Err(_)) => Poll::Ready(Err(Error::Disconnected)),
            Poll::Pending => Poll::Pending,
        }
    }
}
