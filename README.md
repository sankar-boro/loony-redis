# loony redis [![build status](https://github.com/loony-rs/loony-redis/workflows/CI%20%28Linux%29/badge.svg?branch=master&event=push)](https://github.com/loony-rs/loony-redis/actions?query=workflow%3A"CI+(Linux)") [![codecov](https://codecov.io/gh/loony-rs/loony-redis/branch/master/graph/badge.svg)](https://codecov.io/gh/loony-rs/loony-redis) [![crates.io](https://meritbadge.herokuapp.com/loony-redis)](https://crates.io/crates/loony-redis)

redis client for loony framework

## Documentation & community resources

* [Documentation](https://docs.rs/loony-redis)
* Minimum supported Rust version: 1.46 or later

## Example

```rust
use loony_redis::{cmd, RedisConnector};

#[loony::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let redis = RedisConnector::new("127.0.0.1:6379").connect().await?;

    // create list with one value
    redis.exec(cmd::LPush("test", "value"));

    // get value by index
    let value = redis.exec(cmd::LIndex("test", 0)).await?;
    assert_eq!(value.unwrap(), "value");

    // remove key
    redis.exec(cmd::Del("test")).await?;

    Ok(())
}
```

## License

This project is licensed under

* MIT license ([LICENSE](LICENSE) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))
