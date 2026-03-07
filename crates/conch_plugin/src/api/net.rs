use std::net::ToSocketAddrs;
use std::time::Duration;

use mlua::{Lua, Result as LuaResult};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Register the `net` table into the Lua state.
pub fn register(lua: &Lua) -> LuaResult<()> {
    let net = lua.create_table()?;

    // net.check_port(host, port, timeout_ms?) — check if a single port is open.
    // Returns true/false.
    net.set(
        "check_port",
        lua.create_async_function(|_lua, (host, port, timeout_ms): (String, u16, Option<u64>)| {
            async move {
                let dur = Duration::from_millis(timeout_ms.unwrap_or(1000));
                let addr = format!("{host}:{port}");
                let open = timeout(dur, TcpStream::connect(&addr)).await.is_ok_and(|r| r.is_ok());
                Ok(open)
            }
        })?,
    )?;

    // net.scan(host, ports, timeout_ms?, concurrency?) — scan multiple ports.
    // `ports` is a Lua table of port numbers (e.g. {22, 80, 443}).
    // Returns a table of {port=N, open=bool} entries.
    net.set(
        "scan",
        lua.create_async_function(|lua, (host, ports, timeout_ms, concurrency): (String, mlua::Table, Option<u64>, Option<usize>)| {
            async move {
                let dur = Duration::from_millis(timeout_ms.unwrap_or(1000));
                let max_concurrent = concurrency.unwrap_or(50).min(500);

                // Collect port numbers from the Lua table.
                let mut port_list: Vec<u16> = Vec::new();
                for pair in ports.pairs::<mlua::Integer, mlua::Value>() {
                    let (_, val) = pair?;
                    if let mlua::Value::Integer(p) = val {
                        if (1..=65535).contains(&p) {
                            port_list.push(p as u16);
                        }
                    }
                }

                // Scan with bounded concurrency using a semaphore.
                let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
                let host = std::sync::Arc::new(host);

                let mut handles = Vec::with_capacity(port_list.len());
                for port in &port_list {
                    let sem = sem.clone();
                    let host = host.clone();
                    let port = *port;
                    handles.push(tokio::spawn(async move {
                        let _permit = sem.acquire().await;
                        let addr = format!("{host}:{port}");
                        let open = timeout(dur, TcpStream::connect(&addr))
                            .await
                            .is_ok_and(|r| r.is_ok());
                        (port, open)
                    }));
                }

                let result = lua.create_table()?;
                for (i, handle) in handles.into_iter().enumerate() {
                    if let Ok((port, open)) = handle.await {
                        let entry = lua.create_table()?;
                        entry.set("port", port)?;
                        entry.set("open", open)?;
                        result.set(i + 1, entry)?;
                    }
                }
                Ok(result)
            }
        })?,
    )?;

    // net.scan_range(host, start_port, end_port, timeout_ms?, concurrency?) — scan a port range.
    // Returns a table of open port numbers only.
    net.set(
        "scan_range",
        lua.create_async_function(|lua, (host, start, end, timeout_ms, concurrency): (String, u16, u16, Option<u64>, Option<usize>)| {
            async move {
                let dur = Duration::from_millis(timeout_ms.unwrap_or(1000));
                let max_concurrent = concurrency.unwrap_or(100).min(500);

                if start > end || end == 0 {
                    return Err(mlua::Error::runtime("Invalid port range"));
                }

                let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
                let host = std::sync::Arc::new(host);

                let mut handles = Vec::new();
                for port in start..=end {
                    let sem = sem.clone();
                    let host = host.clone();
                    handles.push(tokio::spawn(async move {
                        let _permit = sem.acquire().await;
                        let addr = format!("{host}:{port}");
                        let open = timeout(dur, TcpStream::connect(&addr))
                            .await
                            .is_ok_and(|r| r.is_ok());
                        (port, open)
                    }));
                }

                let result = lua.create_table()?;
                let mut idx = 1;
                for handle in handles {
                    if let Ok((port, open)) = handle.await {
                        if open {
                            result.set(idx, port)?;
                            idx += 1;
                        }
                    }
                }
                Ok(result)
            }
        })?,
    )?;

    // net.resolve(hostname) — DNS lookup. Returns a table of IP address strings.
    net.set(
        "resolve",
        lua.create_async_function(|lua, hostname: String| {
            async move {
                let result = lua.create_table()?;
                // ToSocketAddrs needs a port; use 0.
                let lookup = format!("{hostname}:0");
                match tokio::task::spawn_blocking(move || lookup.to_socket_addrs()).await {
                    Ok(Ok(addrs)) => {
                        let mut idx = 1;
                        let mut seen = std::collections::HashSet::new();
                        for addr in addrs {
                            let ip = addr.ip().to_string();
                            if seen.insert(ip.clone()) {
                                result.set(idx, ip)?;
                                idx += 1;
                            }
                        }
                    }
                    _ => {}
                }
                Ok(result)
            }
        })?,
    )?;

    // net.time() — monotonic timestamp in seconds (for measuring durations).
    net.set(
        "time",
        lua.create_function(|_lua, ()| {
            use std::time::SystemTime;
            let secs = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            Ok(secs)
        })?,
    )?;

    lua.globals().set("net", net)?;
    Ok(())
}
