//! Disposable fixture harness only (`npm run test:mysql-rsa`): TLS-off MySQL sign-in with RSA
//! password exchange against a server whose authentication cache is empty. Run against MySQL
//! 8.4 (`caching_sha2_password` handshake) and MySQL 8.0 started with
//! `--default-authentication-plugin=mysql_native_password`, where every `caching_sha2_password`
//! account is reached through an authentication switch (regression: the switch's nonce must be
//! used for full authentication). Prints only static results; never the password or SQL errors.
use std::{env, time::Duration};
use stoneql_core::Engine;
use stoneql_runtime::{
    Connections, DirectServer, LifecycleError, OwnerId, PasswordRsaKey, QueryLimits,
};

const WRONG_KEY: &[u8] = include_bytes!("../tests/fixtures/rsa-public-key.pem");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = env::var("STONEQL_RSA_HOST")?;
    let port: u16 = env::var("STONEQL_RSA_PORT")?.parse()?;
    let password = env::var("STONEQL_FIXTURE_PASSWORD")?;
    let server_key = std::fs::read(env::var("STONEQL_SERVER_PUBLIC_KEY")?)?;
    let step = env::var("STONEQL_RSA_STEP")?;
    // Idle lifetime longer than the recover step's idle wait (70 s).
    let registry = Connections::new(2, Duration::from_secs(10), Duration::from_secs(600))?;
    let owner = OwnerId::new();
    let config = |user: &'static str, rsa_key: PasswordRsaKey| DirectServer {
        engine: Engine::MySql,
        host: host.clone(),
        port,
        username: user.into(),
        password: password.clone(),
        database: "stoneql_rsa".into(),
        root_certificate_pem: None,
        tls: false,
        rsa_key,
    };
    // One sign-in (pool open plus `SELECT VERSION()`), closed again on success.
    let sign_in = |user: &'static str, rsa_key: PasswordRsaKey| {
        let config = config(user, rsa_key);
        let registry = registry.clone();
        async move {
            let id = registry.connect(owner, config).await?;
            registry.disconnect(owner, id).await?;
            Ok::<(), LifecycleError>(())
        }
    };
    let trusted = || PasswordRsaKey::Trusted(server_key.clone());
    let expect =
        |what: &str, actual: Result<(), LifecycleError>, wanted: Result<(), LifecycleError>| {
            if actual != wanted {
                return Err(format!("{what}: expected {wanted:?}, got {actual:?}"));
            }
            Ok(())
        };
    match step.as_str() {
        // A freshly started server: no account below has signed in yet.
        "first" => {
            expect(
                "no RSA option",
                sign_in("rsa_none", PasswordRsaKey::None).await,
                Err(LifecycleError::EncryptedLoginRequired),
            )?;
            expect(
                "wrong trusted key",
                sign_in("rsa_wrong", PasswordRsaKey::Trusted(WRONG_KEY.to_vec())).await,
                Err(LifecycleError::Unavailable),
            )?;
            expect(
                "trusted key",
                sign_in("rsa_trusted", trusted()).await,
                Ok(()),
            )?;
            expect(
                "reconnect (cached)",
                sign_in("rsa_trusted", PasswordRsaKey::None).await,
                Ok(()),
            )?;
            expect(
                "retrieval",
                sign_in("rsa_retrieve", PasswordRsaKey::RetrieveUnverified).await,
                Ok(()),
            )?;
            expect(
                "reconnect with the trusted key",
                sign_in("rsa_retrieve", trusted()).await,
                Ok(()),
            )?;
            expect(
                "REQUIRE SSL account",
                sign_in("rsa_ssl", PasswordRsaKey::RetrieveUnverified).await,
                Err(LifecycleError::Unavailable),
            )?;
        }
        // The same server restarted (same data directory and key pair): the cache is empty again
        // and the key saved before the restart still works.
        "after-restart" => {
            expect(
                "cache emptied by the restart",
                sign_in("rsa_trusted", PasswordRsaKey::None).await,
                Err(LifecycleError::EncryptedLoginRequired),
            )?;
            expect(
                "key saved before the restart",
                sign_in("rsa_trusted", trusted()).await,
                Ok(()),
            )?;
            expect(
                "reconnect (cached)",
                sign_in("rsa_trusted", PasswordRsaKey::None).await,
                Ok(()),
            )?;
            expect(
                "retrieval",
                sign_in("rsa_retrieve", PasswordRsaKey::RetrieveUnverified).await,
                Ok(()),
            )?;
        }
        // One handle kept across an idle period and a server restart (the orchestrator checks
        // the server holds no connection for it while idle, then restarts the server between
        // the two queries). The second query must succeed on the same handle: the host reopens
        // the connection with the sign-in details it keeps, including the full RSA sign-in.
        "recover" => {
            let limits = || QueryLimits {
                rows: 1,
                bytes: 4096,
                deadline: Duration::from_secs(10),
            };
            let id = registry
                .connect(owner, config("rsa_trusted", trusted()))
                .await
                .map_err(|error| format!("connect: {error:?}"))?;
            registry
                .read_only(owner, id, "SELECT 1", limits())
                .await
                .map_err(|error| format!("first query: {error:?}"))?;
            println!("HELD");
            std::io::stdin().read_line(&mut String::new())?;
            if !registry.is_connected(owner, id) {
                return Err("the handle ended while idle".into());
            }
            registry
                .read_only(owner, id, "SELECT 1", limits())
                .await
                .map_err(|error| format!("query after idle and restart: {error:?}"))?;
            registry.disconnect(owner, id).await?;
        }
        _ => return Err("unknown step".into()),
    }
    registry.shutdown().await;
    println!("{{\"step\":\"{step}\",\"passed\":true}}");
    Ok(())
}
