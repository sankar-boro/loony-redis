//! Redis client for loony framework.
//!
//! # Example
//!
//! ```rust
//! use loony_redis::{cmd, RedisConnector};
//! # use rand::{thread_rng, Rng};
//! # use rand::distributions::Alphanumeric;
//! # fn gen_random_key() -> String {
//! # let key: String = thread_rng().sample_iter(&Alphanumeric).take(12).map(char::from).collect();
//! # key
//! # }
//!
//! #[loony::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let redis = RedisConnector::new("127.0.0.1:6379").connect().await?;
//!     let key = gen_random_key();
//!
//!     // create list with one value
//!     redis.exec(cmd::LPush(&key, "value"));
//!
//!     // get value by index
//!     let value = redis.exec(cmd::LIndex(&key, 0)).await?;
//!     assert_eq!(value.unwrap(), "value");
//!
//!     // remove key
//!     redis.exec(cmd::Del(&key)).await?;
//!
//!     Ok(())
//! }
//! ```
mod client;
pub mod cmd;
pub mod codec;
mod connector;
pub mod errors;
mod simple;
mod session;
mod redis;
// mod transport;

pub use self::client::{Client, CommandResult};
pub use self::connector::RedisConnector;
pub use self::simple::SimpleClient;

/// Macro to create a request array, useful for preparing commands to send. Elements can be any type, or a mixture
/// of types, that satisfy `Into<Request>`.
///
/// # Examples
///
/// ```rust
/// #[macro_use]
/// extern crate loony_redis;
///
/// fn main() {
///     let value = format!("something_{}", 123);
///     array!["SET", "key_name", value];
/// }
/// ```
///
/// For variable length Redis commands:
///
/// ```rust
/// #[macro_use]
/// extern crate loony_redis;
///
/// fn main() {
///     let data = vec!["data", "from", "somewhere", "else"];
///     let command = array!["RPUSH", "mykey"].extend(data);
/// }
/// ```
#[macro_export]
macro_rules! array {
    ($($e:expr),*) => {{
        $crate::codec::Request::Array(vec![$($e.into(),)*])
    }}
}

#[cfg(test)]
pub fn gen_random_key() -> String {
    use rand::distributions::Alphanumeric;
    use rand::{thread_rng, Rng};

    let key: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect();
    key
}

pub use redis::{Command, RedisActor};
use derive_more::{Display, Error, From};

#[cfg(feature = "web")]
pub use loony::cookie::SameSite;
#[cfg(feature = "web")]
pub use session::RedisSession;

/// General purpose actix redis error
#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Redis error {}", _0)]
    Redis(redis_async::error::Error),
    /// Receiving message during reconnecting
    #[display(fmt = "Redis: Not connected")]
    NotConnected,
    /// Cancel all waters when connection get dropped
    #[display(fmt = "Redis: Disconnected")]
    Disconnected,
}

#[cfg(feature = "web")]
impl loony::http::ResponseError for Error {}

// re-export
pub use redis_async::error::Error as RespError;
pub use redis_async::resp::RespValue;