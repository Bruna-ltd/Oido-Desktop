//! MCP servers: what is registered, what each last offered, and its sealed secrets.

use rusqlite::OptionalExtension;

use super::{Result, Store};
use crate::mcp::{Auth, Secrets, Server, Status, Tool, Transport};

impl Store {
    pub fn load_mcp_servers(&self) -> Result<Vec<Server>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListMcpServers")?)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
            ))
        })?;
        let mut servers = Vec::new();
        for row in rows {
            let (id, name, transport, url, command, args, auth, enabled, status, detail, tools) =
                row?;
            servers.push(Server {
                transport: if transport == "stdio" {
                    Transport::Stdio {
                        command,
                        args: serde_json::from_str(&args)?,
                    }
                } else {
                    Transport::Http { url }
                },
                auth: Auth::parse(&auth),
                enabled: enabled != 0,
                status: Status::parse(&status),
                detail,
                tools: serde_json::from_str(&tools)?,
                id,
                name,
            });
        }
        Ok(servers)
    }

    /// Registers a server with its secrets, sealed. Nothing is written if they cannot be sealed.
    pub fn insert_mcp_server(&self, server: &Server, secrets: &Secrets) -> Result<()> {
        let sealed = self.seal_mcp_secrets(&server.id, secrets)?;
        let (transport, url, command, args) = columns(&server.transport)?;
        self.conn
            .prepare_cached(self.sql("InsertMcpServer")?)?
            .execute((
                &server.id,
                &server.name,
                transport,
                url,
                command,
                args,
                server.auth.as_str(),
                i64::from(server.enabled),
                server.status.as_str(),
                &server.detail,
                serde_json::to_string(&server.tools)?,
                sealed,
            ))?;
        Ok(())
    }

    /// Saves how the server is reached and signed in to. Its name, state, tools and secrets are
    /// saved apart.
    pub fn update_mcp_server(&self, server: &Server) -> Result<()> {
        let (transport, url, command, args) = columns(&server.transport)?;
        self.conn
            .prepare_cached(self.sql("UpdateMcpServer")?)?
            .execute((
                &server.id,
                transport,
                url,
                command,
                args,
                server.auth.as_str(),
            ))?;
        Ok(())
    }

    pub fn delete_mcp_server(&self, id: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteMcpServer")?)?
            .execute([id])?;
        Ok(())
    }

    pub fn set_mcp_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("SetMcpEnabled")?)?
            .execute((id, i64::from(enabled)))?;
        Ok(())
    }

    /// What the last attempt to list the server's tools found. A failure keeps the tools it had.
    pub fn save_mcp_listing(
        &self,
        id: &str,
        status: Status,
        detail: &str,
        tools: &[Tool],
    ) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("SetMcpListing")?)?
            .execute((id, status.as_str(), detail, serde_json::to_string(tools)?))?;
        Ok(())
    }

    /// The server's secrets, opened. Empty when it has none.
    pub fn mcp_secrets(&self, id: &str) -> Result<Secrets> {
        let sealed: Option<Option<Vec<u8>>> = self
            .conn
            .prepare_cached(self.sql("GetMcpSecrets")?)?
            .query_row([id], |r| r.get(0))
            .optional()?;
        match sealed.flatten() {
            None => Ok(Secrets::default()),
            Some(blob) => {
                let plain = self.vault.open(&secrets_context(id), &blob)?;
                Ok(serde_json::from_str(&plain)?)
            }
        }
    }

    /// Replaces the server's secrets, such as after its sign-in tokens were renewed.
    pub fn save_mcp_secrets(&self, id: &str, secrets: &Secrets) -> Result<()> {
        let sealed = self.seal_mcp_secrets(id, secrets)?;
        self.conn
            .prepare_cached(self.sql("SetMcpSecrets")?)?
            .execute((id, sealed))?;
        Ok(())
    }

    fn seal_mcp_secrets(&self, id: &str, secrets: &Secrets) -> Result<Vec<u8>> {
        let json = zeroize::Zeroizing::new(serde_json::to_string(secrets)?);
        Ok(self.vault.seal(&secrets_context(id), &json)?)
    }
}

/// The `transport`, `url`, `command` and `args` columns.
fn columns(t: &Transport) -> Result<(&'static str, &str, &str, String)> {
    Ok(match t {
        Transport::Http { url } => ("http", url.as_str(), "", "[]".to_owned()),
        Transport::Stdio { command, args } => {
            ("stdio", "", command.as_str(), serde_json::to_string(args)?)
        }
    })
}

/// What sealed secrets are bound to, so one server's cannot be moved to another's row.
fn secrets_context(id: &str) -> String {
    format!("mcp-secrets:{id}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::tests::mem;
    use crate::mcp::{Auth, Secrets, Server, Status, Tool, Transport};

    fn server(id: &str, name: &str, transport: Transport) -> Server {
        Server {
            id: id.into(),
            name: name.into(),
            transport,
            auth: Auth::Key,
            enabled: true,
            status: Status::Pending,
            detail: String::new(),
            tools: Vec::new(),
        }
    }

    #[test]
    fn servers_keep_their_order_listing_and_sealed_secrets() {
        let s = mem();
        let secrets = Secrets {
            key: Some("sk-secret".into()),
            ..Secrets::default()
        };
        s.insert_mcp_server(
            &server(
                "a",
                "Docs",
                Transport::Http {
                    url: "https://x/mcp".into(),
                },
            ),
            &secrets,
        )
        .unwrap();
        s.insert_mcp_server(
            &server(
                "b",
                "Local",
                Transport::Stdio {
                    command: "npx".into(),
                    args: vec!["-y".into(), "pkg".into()],
                },
            ),
            &Secrets::default(),
        )
        .unwrap();
        let tool = Tool {
            name: "search".into(),
            description: "Searches.".into(),
            schema: serde_json::json!({"type": "object"}),
        };
        s.save_mcp_listing("a", Status::Reachable, "", std::slice::from_ref(&tool))
            .unwrap();
        s.save_mcp_listing("b", Status::Unreachable, "could not start", &[])
            .unwrap();
        s.set_mcp_enabled("b", false).unwrap();

        let list = s.load_mcp_servers().unwrap();
        assert_eq!(
            list.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            ["Docs", "Local"]
        );
        assert_eq!(list[0].tools, [tool]);
        assert_eq!(list[0].status, Status::Reachable);
        assert!(!list[1].enabled);
        assert_eq!(list[1].detail, "could not start");
        assert!(
            matches!(&list[1].transport, Transport::Stdio { args, .. } if args == &["-y", "pkg"])
        );

        assert_eq!(
            s.mcp_secrets("a").unwrap().key.as_deref(),
            Some("sk-secret")
        );
        let raw: Vec<u8> = s
            .conn
            .query_row(
                "SELECT secrets_enc FROM mcp_servers WHERE id = 'a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&raw).contains("sk-secret"),
            "the key is sealed"
        );

        s.save_mcp_secrets("a", &Secrets::default()).unwrap();
        assert_eq!(s.mcp_secrets("a").unwrap().key, None);
        s.delete_mcp_server("a").unwrap();
        assert_eq!(s.load_mcp_servers().unwrap().len(), 1);
    }

    #[test]
    fn a_name_is_taken_once_and_secrets_do_not_open_on_another_row() {
        let s = mem();
        let http = || Transport::Http {
            url: "https://x/mcp".into(),
        };
        s.insert_mcp_server(&server("a", "Docs", http()), &Secrets::default())
            .unwrap();
        assert!(
            s.insert_mcp_server(&server("b", "Docs", http()), &Secrets::default())
                .is_err()
        );
        let sealed = s.seal_mcp_secrets("a", &Secrets::default()).unwrap();
        assert!(s.vault.open("mcp-secrets:b", &sealed).is_err());
    }
}
