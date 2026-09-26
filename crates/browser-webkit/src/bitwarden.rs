use std::path::PathBuf;
use std::process::Command;

use aegis_url_adapter::WebUrl;
use serde::Deserialize;

const BITWARDEN_SERVER: &str = "https://vault.bitwarden.eu";

#[derive(Debug, Clone)]
pub(crate) struct Login {
    pub name: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
struct VaultItem {
    id: String,
    name: String,
    login: Option<LoginData>,
}

#[derive(Debug, Deserialize)]
struct LoginData {
    username: Option<String>,
    password: Option<String>,
    uris: Option<Vec<LoginUri>>,
}

#[derive(Debug, Deserialize)]
struct LoginUri {
    uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    status: String,
}

pub(crate) fn list_logins(origin: &str) -> Result<Vec<Login>, String> {
    let status: StatusResponse = serde_json::from_str(&run_bw(&["status"])?)
        .map_err(|_| "resposta inválida do Bitwarden CLI".to_owned())?;
    if status.status != "unlocked" {
        return Err(format!(
            "Bitwarden não está desbloqueado (estado: {}). Configure `bw config server https://vault.bitwarden.eu`, execute `bw login`, depois `bw unlock` e exporte BW_SESSION.",
            status.status
        ));
    }

    // Bitwarden's text search is not guaranteed to search login URIs. Read the
    // unlocked item list and apply the URI match locally instead.
    let listed: Vec<VaultItem> = serde_json::from_str(&run_bw(&["list", "items"])?)
        .map_err(|_| "não foi possível ler os itens do Bitwarden".to_owned())?;
    let mut logins = Vec::new();
    for item in listed {
        let Some(login) = item.login else {
            continue;
        };
        let uri_matches = login
            .uris
            .as_ref()
            .map(|uris| {
                uris.iter()
                    .filter_map(|uri| uri.uri.as_deref())
                    .any(|uri| uri_matches_origin(uri, origin))
            })
            .unwrap_or(false);
        let (Some(username), Some(password)) = (login.username, login.password) else {
            continue;
        };
        let _ = item.id;
        logins.push((
            !uri_matches,
            Login {
                name: item.name,
                username,
                password,
            },
        ));
    }
    logins.sort_by(|(left_not_match, left), (right_not_match, right)| {
        left_not_match
            .cmp(right_not_match)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(logins.into_iter().map(|(_, login)| login).collect())
}

fn uri_matches_origin(uri: &str, origin: &str) -> bool {
    let Ok(stored_url) = WebUrl::parse(uri.trim()) else {
        return false;
    };
    stored_url.origin().canonical_string() == origin.trim_end_matches('/')
}

pub(crate) fn autofill_script(login: &Login) -> String {
    let username = serde_json::to_string(&login.username).unwrap_or_else(|_| "\"\"".to_owned());
    let password = serde_json::to_string(&login.password).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        r#"(function() {{
            const username = {username};
            const password = {password};
            const startedAt = Date.now();
            function visible(input) {{
                return input && input.getClientRects().length > 0 &&
                    !input.disabled && !input.readOnly;
            }}
            function findFields() {{
                const inputs = Array.from(document.querySelectorAll('input, textarea'))
                    .filter(visible);
                const passwordInput = inputs.find((input) => input.type === 'password');
                const userInput = inputs.find((input) =>
                    /user(name)?|email|login|account|identifier/i.test(
                        [input.name, input.id, input.autocomplete, input.placeholder,
                         input.getAttribute('aria-label')].filter(Boolean).join(' ')
                    )
                ) || inputs.find((input) => input.type === 'email' || input.type === 'text');
                return {{ userInput, passwordInput }};
            }}
            function setValue(input, value) {{
                if (!input) return;
                input.focus();
                const prototype = Object.getPrototypeOf(input);
                const descriptor = Object.getOwnPropertyDescriptor(prototype, 'value') ||
                    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
                if (descriptor && descriptor.set) descriptor.set.call(input, value);
                else input.value = value;
                input.dispatchEvent(new Event('input', {{ bubbles: true }}));
                input.dispatchEvent(new Event('change', {{ bubbles: true }}));
            }}
            function tryFill() {{
                const {{ userInput, passwordInput }} = findFields();
                setValue(userInput, username);
                setValue(passwordInput, password);
                if ((!userInput && !passwordInput) && Date.now() - startedAt < 3000) {{
                    window.setTimeout(tryFill, 150);
                }}
            }}
            tryFill();
        }})();"#
    )
}

fn run_bw(args: &[&str]) -> Result<String, String> {
    let output = Command::new(bitwarden_command())
        .env("BW_SERVER", BITWARDEN_SERVER)
        .args(args)
        .output()
        .map_err(|_| "Bitwarden CLI não encontrado no PATH".to_owned())?;
    if !output.status.success() {
        return Err(
            "Bitwarden CLI recusou a operação; verifique se o cofre está desbloqueado".to_owned(),
        );
    }
    String::from_utf8(output.stdout)
        .map_err(|_| "Bitwarden CLI retornou dados inválidos".to_owned())
}

fn bitwarden_command() -> PathBuf {
    if let Some(path) = std::env::var_os("AEGIS_BITWARDEN_BIN") {
        return PathBuf::from(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let local_bin = PathBuf::from(home).join(".local/bin/bw");
        if local_bin.is_file() {
            return local_bin;
        }
    }
    PathBuf::from("bw")
}
