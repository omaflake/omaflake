# omaflake-sdk

[omaflake](https://omaflake.dev) – clean, disposable NixOS and macOS VMs on Apple silicon and Linux, declared in Nix ("recipes"), started from a menu bar app or the command line `omaf`.

Talk to omaflake from your own program: the client of its service (`omaf serve`, a JSON-RPC socket), the API's
types, its errors and texts – what omaflake's own interfaces use.

```rust
use omaflake_sdk::client::Client;

let omaf = Client::connect(std::path::Path::new("/Users/me/.omaflake/run/omaf.sock"))?; // in a fn returning Result
let session = omaf.initialize("my-tool")?;
println!("omaflake {} on {}", session.omaf_version, session.host.os);
let vms = omaf.call("vms.list", serde_json::Value::Null)?;
```

It needs a running omaflake: the app, or `omaf service start`. The API: <https://omaflake.dev/api/>. The session, jobs,
events, errors and texts are here now; the other types and the page models follow.

Licence: AGPL-3.0-or-later. Source and issues: <https://github.com/omaflake/omaflake>. All of omaflake's crates are released together, with omaflake's version.
