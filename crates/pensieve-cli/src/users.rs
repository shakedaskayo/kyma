//! `pensieve user …` — admin user management against `/v1/admin/users`.
//!
//! These talk to a running server (not Postgres directly) and require an
//! **admin** bearer token saved via `pensieve connect <url> --token <admin-token>`.
//! Passwords are sent to the server, which hashes them (argon2id) — the CLI
//! never hashes or stores them.

use std::io::Read;

use anyhow::{anyhow, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::client::{
    delete_json, effective_config, get_json, patch_json, post_json, ClientConfig,
};

#[derive(Debug, Subcommand)]
pub(crate) enum UsersOp {
    /// List all users.
    List,
    /// Create a new user (prompts for a password unless --password-stdin).
    Create {
        username: String,
        /// Role: read, write, or admin.
        #[arg(long, default_value = "read")]
        role: String,
        /// Read the password from stdin instead of prompting interactively.
        #[arg(long)]
        password_stdin: bool,
    },
    /// Reset a user's password (prompts unless --password-stdin).
    Passwd {
        username: String,
        #[arg(long)]
        password_stdin: bool,
    },
    /// Change a user's role (read | write | admin).
    SetRole { username: String, role: String },
    /// Delete a user.
    Delete {
        username: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

pub(crate) async fn run(op: UsersOp) -> Result<()> {
    let cfg = effective_config()?;
    require_token(&cfg)?;

    match op {
        UsersOp::List => {
            let users = hint_auth(get_json(&cfg, "/v1/admin/users").await)?;
            print_users(&users);
        }
        UsersOp::Create {
            username,
            role,
            password_stdin,
        } => {
            validate_role(&role)?;
            let password = read_password(password_stdin)?;
            let user = hint_auth(
                post_json(
                    &cfg,
                    "/v1/admin/users",
                    json!({ "username": username, "password": password, "role": role }),
                )
                .await,
            )?;
            println!("created user {} ({})", field(&user, "username"), field(&user, "role"));
        }
        UsersOp::Passwd {
            username,
            password_stdin,
        } => {
            let password = read_password(password_stdin)?;
            hint_auth(
                patch_json(
                    &cfg,
                    &format!("/v1/admin/users/{username}"),
                    json!({ "password": password }),
                )
                .await,
            )?;
            println!("password updated for {username}");
        }
        UsersOp::SetRole { username, role } => {
            validate_role(&role)?;
            hint_auth(
                patch_json(
                    &cfg,
                    &format!("/v1/admin/users/{username}"),
                    json!({ "role": role }),
                )
                .await,
            )?;
            println!("{username} is now {role}");
        }
        UsersOp::Delete { username, yes } => {
            if !yes && !confirm(&format!("Delete user {username}? [y/N] "))? {
                println!("aborted");
                return Ok(());
            }
            hint_auth(delete_json(&cfg, &format!("/v1/admin/users/{username}")).await)?;
            println!("deleted user {username}");
        }
    }
    Ok(())
}

fn require_token(cfg: &ClientConfig) -> Result<()> {
    if cfg.token.is_none() {
        return Err(anyhow!(
            "user management needs an admin token — run `pensieve connect {} --token <admin-token>`",
            cfg.endpoint
        ));
    }
    Ok(())
}

fn validate_role(role: &str) -> Result<()> {
    match role {
        "read" | "write" | "admin" => Ok(()),
        other => Err(anyhow!("invalid role `{other}` (expected: read, write, admin)")),
    }
}

/// Read a password from stdin (when piped) or via a hidden interactive prompt
/// (with confirmation).
fn read_password(from_stdin: bool) -> Result<String> {
    if from_stdin {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| anyhow!("reading password from stdin: {e}"))?;
        let pw = buf.trim_end_matches(['\n', '\r']).to_string();
        if pw.is_empty() {
            return Err(anyhow!("empty password on stdin"));
        }
        Ok(pw)
    } else {
        let pw = rpassword::prompt_password("Password: ")
            .map_err(|e| anyhow!("reading password: {e}"))?;
        let confirm = rpassword::prompt_password("Confirm password: ")
            .map_err(|e| anyhow!("reading password: {e}"))?;
        if pw != confirm {
            return Err(anyhow!("passwords do not match"));
        }
        Ok(pw)
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

/// Add an actionable hint when the server rejects us for auth reasons.
fn hint_auth(result: Result<Value>) -> Result<Value> {
    result.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("401") {
            anyhow!("{msg}\n  → not authenticated; run `pensieve connect <url> --token <admin-token>`")
        } else if msg.contains("403") {
            anyhow!("{msg}\n  → your token is not an admin token; user management requires the admin role")
        } else {
            e
        }
    })
}

fn print_users(users: &Value) {
    let Some(arr) = users.as_array() else {
        println!("{users}");
        return;
    };
    if arr.is_empty() {
        println!("(no users)");
        return;
    }
    println!("{:<24} {:<8} {}", "USERNAME", "ROLE", "CREATED");
    for u in arr {
        println!(
            "{:<24} {:<8} {}",
            field(u, "username"),
            field(u, "role"),
            field(u, "created_at"),
        );
    }
}

fn field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("-")
}
