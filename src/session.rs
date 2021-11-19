use std::{collections::HashMap, iter, rc::Rc};

use actix::prelude::*;
use futures::task::Poll;
use futures::future::LocalBoxFuture;
use loony::service::{Service, Transform};
use loony::web::{WebRequest, WebResponse};
use loony_session::{Session, SessionStatus};
use rand::Rng;
use rand::distributions::Alphanumeric;
use rand::rngs::OsRng;
use redis_async::resp_array;
use time::{self, Duration, OffsetDateTime};
use loony::http::{self, HeaderValue, header, StatusCode};
use loony::web::error::{self, Error};
use loony::cookie::{Cookie, CookieJar, Key, SameSite};
use redis_async::resp::{RespCodec, RespValue};
use crate::RedisActor;
use crate::redis::Command;
use derive_more::Display;
use loony::web::{DefaultError, ErrorRenderer, HttpResponse, WebResponseError};

/// A set of errors that can occur during processing CORS
#[derive(Debug, Display)]
pub enum RedisError {
    /// The HTTP request header `Origin` is required but was not provided
    #[display(fmt = "The HTTP request header `Origin` is required but was not provided")]
    MissingOrigin,
    /// The HTTP request header `Origin` could not be parsed correctly.
    #[display(fmt = "The HTTP request header `Origin` could not be parsed correctly.")]
    BadOrigin,
}

/// DefaultError renderer support
impl WebResponseError<DefaultError> for RedisError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}


/// Use redis as session storage.
///
/// You need to pass an address of the redis server and random value to the
/// constructor of `RedisSession`. This is private key for cookie
/// session, When this value is changed, all session data is lost.
///
/// Constructor panics if key length is less than 32 bytes.
pub struct RedisSession(Rc<Inner>);

impl RedisSession {
    /// Create new redis session backend
    ///
    /// * `addr` - address of the redis server
    pub fn new<S: Into<String>>(addr: S, key: &[u8]) -> RedisSession {
        RedisSession(Rc::new(Inner {
            key: Key::derive_from(key),
            cache_keygen: Box::new(|key: &str| format!("session:{}", &key)),
            ttl: "7200".to_owned(),
            addr: RedisActor::start(addr),
            name: "actix-session".to_owned(),
            path: "/".to_owned(),
            domain: None,
            secure: false,
            max_age: Some(Duration::days(7)),
            same_site: None,
            http_only: true,
        }))
    }

    /// Set time to live in seconds for session value.
    pub fn ttl(mut self, ttl: u32) -> Self {
        Rc::get_mut(&mut self.0).unwrap().ttl = format!("{}", ttl);
        self
    }


    /// Set custom cookie name for session ID.
    pub fn cookie_name(mut self, name: &str) -> Self {
        Rc::get_mut(&mut self.0).unwrap().name = name.to_owned();
        self
    }

    /// Set custom cookie path.
    pub fn cookie_path(mut self, path: &str) -> Self {
        Rc::get_mut(&mut self.0).unwrap().path = path.to_owned();
        self
    }

    /// Set custom cookie domain.
    pub fn cookie_domain(mut self, domain: &str) -> Self {
        Rc::get_mut(&mut self.0).unwrap().domain = Some(domain.to_owned());
        self
    }

    /// Set custom cookie secure.
    ///
    /// If the `secure` field is set, a cookie will only be transmitted when the
    /// connection is secure - i.e. `https`.
    ///
    /// Default is false.
    pub fn cookie_secure(mut self, secure: bool) -> Self {
        Rc::get_mut(&mut self.0).unwrap().secure = secure;
        self
    }

    /// Set custom cookie max-age.
    ///
    /// Use `None` for session-only cookies.
    pub fn cookie_max_age(mut self, max_age: impl Into<Option<Duration>>) -> Self {
        Rc::get_mut(&mut self.0).unwrap().max_age = max_age.into();
        self
    }

    /// Set custom cookie `SameSite` attribute.
    ///
    /// By default, the attribute is omitted.
    pub fn cookie_same_site(mut self, same_site: SameSite) -> Self {
        Rc::get_mut(&mut self.0).unwrap().same_site = Some(same_site);
        self
    }

    /// Set custom cookie `HttpOnly` policy.
    ///
    /// Default is true.
    pub fn cookie_http_only(mut self, http_only: bool) -> Self {
        Rc::get_mut(&mut self.0).unwrap().http_only = http_only;
        self
    }

    /// Set a custom cache key generation strategy, expecting session key as input.
    pub fn cache_keygen(mut self, keygen: Box<dyn Fn(&str) -> String>) -> Self {
        Rc::get_mut(&mut self.0).unwrap().cache_keygen = keygen;
        self
    }
}

impl<S, Err> Transform<S> for RedisSession
where
    S: Service<Request=WebRequest<Err>, Response=WebResponse> + 'static,
    S::Future: 'static,
    S::Error: 'static,
{
    // type Response = WebResponse<B>;
    // type Error = S::Error;
    // type Transform = RedisSessionMiddleware<S>;
    // type InitError = ();
    // type Future = LocalBoxFuture<'static, Result<Self::Transform, Self::InitError>>;
    type Service = RedisSessionMiddleware<S>;

    fn new_transform(&self, service: S) -> Self::Service {
        let inner = self.0.clone();
        // Box::pin(async {
        //     Ok(RedisSessionMiddleware {
        //         service: Rc::new(service),
        //         inner,
        //     })
        // })
        RedisSessionMiddleware {
            service: Rc::new(service),
            inner,
        }
    }
}

/// Cookie session middleware
pub struct RedisSessionMiddleware<S: 'static> {
    service: Rc<S>,
    inner: Rc<Inner>,
}

impl<S, Err> Service for RedisSessionMiddleware<S>
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
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn call(&self, mut req: WebRequest<Err>) -> Self::Future {
        todo!()
        // let srv = Rc::clone(&self.service);
        // let inner = Rc::clone(&self.inner);

        // Box::pin(async move {
        //     let state = inner.load(&req).await?;

        //     let value = if let Some((state, value)) = state {
        //         Session::set_session(state, &mut req);
        //         Some(value)
        //     } else {
        //         None
        //     };

        //     let mut res = srv.call(req).await?;

        //     match Session::get_changes(&mut res) {
        //         (SessionStatus::Unchanged, state) => {
        //             if value.is_none() {
        //                 // implies the session is new
        //                 inner.update(res, state, value).await
        //             } else {
        //                 Ok(res)
        //             }
        //         }

        //         (SessionStatus::Changed, state) => inner.update(res, state, value).await,

        //         (SessionStatus::Purged, _) => {
        //             if let Some(val) = value {
        //                 inner.clear_cache(val).await?;
        //                 match inner.remove_cookie(&mut res) {
        //                     Ok(_) => Ok(res),
        //                     Err(_err) => Err(error::ErrorInternalServerError(_err)),
        //                 }
        //             } else {
        //                 Err(error::ErrorInternalServerError("unexpected"))
        //             }
        //         }

        //         (SessionStatus::Renewed, state) => {
        //             if let Some(val) = value {
        //                 inner.clear_cache(val).await?;
        //                 inner.update(res, state, None).await
        //             } else {
        //                 inner.update(res, state, None).await
        //             }
        //         }
        //     }
        // })
    }
    
    fn poll_ready(&self, _: &mut std::task::Context<'_>) -> Poll<Result<(), <Self as loony::Service>::Error>> { 
        todo!() 
    }
}

struct Inner {
    key: Key,
    cache_keygen: Box<dyn Fn(&str) -> String>,
    ttl: String,
    addr: Addr<RedisActor>,
    name: String,
    path: String,
    domain: Option<String>,
    secure: bool,
    max_age: Option<Duration>,
    same_site: Option<SameSite>,
    http_only: bool,
}

impl Inner {
    async fn load(
        &self,
        // req: &WebRequest,
    ) -> Result<Option<(HashMap<String, String>, String)>, Error> {
        todo!()
        // wrapped in block to avoid holding `Ref` (from `req.cookies`) across await point
        // let (value, cache_key) = {

        //     let cookies = if let Ok(cookies) = req.cookies() {
        //         cookies
        //     } else {
        //         return Ok(None);
        //     };

        //     if let Some(cookie) = cookies.iter().find(|&cookie| cookie.name() == self.name) {
        //         let mut jar = CookieJar::new();
        //         jar.add_original(cookie.clone());

        //         if let Some(cookie) = jar.signed(&self.key).get(&self.name) {
        //             let value = cookie.value().to_owned();
        //             let cache_key = (self.cache_keygen)(cookie.value());
        //             (value, cache_key)
        //         } else {
        //             return Ok(None);
        //         }
        //     } else {
        //         return Ok(None);
        //     }
        // };

        // let val = self
        //     .addr
        //     .send(Command(resp_array!["GET", cache_key]))
        //     .await
        //     .map_err(error::ErrorInternalServerError)?
        //     .map_err(error::ErrorInternalServerError)?;

        // match val {
        //     RespValue::Error(err) => {
        //         // return Err(Some((err, err)));
        //         todo!()
        //     }
        //     RespValue::SimpleString(s) => {
        //         if let Ok(val) = serde_json::from_str(&s) {
        //             return Ok(Some((val, value)));
        //         }
        //     }
        //     RespValue::BulkString(s) => {
        //         if let Ok(val) = serde_json::from_slice(&s) {
        //             return Ok(Some((val, value)));
        //         }
        //     }
        //     _ => {}
        // }

        // Ok(None)
    }

    async fn update<B>(
        &self,
        mut res: WebResponse,
        state: impl Iterator<Item = (String, String)>,
        value: Option<String>,
    ) -> Result<WebResponse, Error> {
        // let (value, jar) = if let Some(value) = value {
        //     (value, None)
        // } else {
        //     let value = iter::repeat(())
        //         .map(|()| OsRng.sample(Alphanumeric))
        //         .take(32)
        //         .collect::<Vec<_>>();
        //     let value = String::from_utf8(value).unwrap_or_default();

        //     // prepare session id cookie
        //     let mut cookie = Cookie::new(self.name.clone(), value.clone());
        //     cookie.set_path(self.path.clone());
        //     cookie.set_secure(self.secure);
        //     cookie.set_http_only(self.http_only);

        //     if let Some(ref domain) = self.domain {
        //         cookie.set_domain(domain.clone());
        //     }

        //     if let Some(max_age) = self.max_age {
        //         cookie.set_max_age(max_age);
        //     }

        //     if let Some(same_site) = self.same_site {
        //         cookie.set_same_site(same_site);
        //     }

        //     // set cookie
        //     let mut jar = CookieJar::new();
        //     jar.signed_mut(&self.key).add(cookie);

        //     (value, Some(jar))
        // };

        // let cache_key = (self.cache_keygen)(&value);

        // let state: HashMap<_, _> = state.collect();

        // let body = match serde_json::to_string(&state) {
        //     Err(err) => return Err(err.into()),
        //     Ok(body) => body,
        // };

        // let cmd = Command(resp_array!["SET", cache_key, body, "EX", &self.ttl]);

        // self.addr
        //     .send(cmd)
        //     .await
        //     .map_err(error::ErrorInternalServerError)?
        //     .map_err(error::ErrorInternalServerError)?;

        // if let Some(jar) = jar {
        //     for cookie in jar.delta() {
        //         let val = HeaderValue::from_str(&cookie.to_string())?;
        //         res.headers_mut().append(header::SET_COOKIE, val);
        //     }
        // }

        // Ok(res)
        todo!()
    }

    /// Removes cache entry.
    async fn clear_cache(&self, key: String) -> Result<(), Error> {
        let cache_key = (self.cache_keygen)(&key);

        let res = self
            .addr
            .send(Command(resp_array!["DEL", cache_key]))
            .await
            .map_err(error::ErrorInternalServerError)?;

        // match res {
        //     // redis responds with number of deleted records
        //     Ok(RespValue::Integer(x)) if x > 0 => Ok(()),
        //     _ => Err(error::ErrorInternalServerError(
        //         "failed to remove session from cache",
        //     )),
        // }
        todo!()
    }

    /// Invalidates session cookie.
    fn remove_cookie<B>(&self, res: &mut WebResponse) -> Result<(), Error> {
        // let mut cookie = Cookie::named(self.name.clone());
        // cookie.set_value("");
        // cookie.set_max_age(Duration::zero());
        // cookie.set_expires(OffsetDateTime::now_utc() - Duration::days(365));

        // let val =
        //     HeaderValue::from_str(&cookie.to_string()).map_err(error::ErrorInternalServerError)?;
        // res.headers_mut().append(header::SET_COOKIE, val);

        // Ok(())
        todo!()
    }
}
