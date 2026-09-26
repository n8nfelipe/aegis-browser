//! Adaptador de URL entre a engine e o núcleo de políticas.
//!
//! O crate `url` implementa o parsing de URL. Este adaptador restringe o
//! resultado às origens web suportadas pelo MVP e não permite que userinfo
//! atravesse a fronteira de segurança.

use aegis_policy_core::{Origin, OriginError, Scheme};
use url::Url;

#[derive(Debug, Clone)]
pub struct WebUrl {
    parsed: Url,
    origin: Origin,
}

impl WebUrl {
    pub fn parse(input: &str) -> Result<Self, UrlAdapterError> {
        let parsed = Url::parse(input).map_err(UrlAdapterError::Parse)?;

        let scheme = match parsed.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err(UrlAdapterError::UnsupportedScheme),
        };

        if parsed.host_str().is_none() {
            return Err(UrlAdapterError::MissingHost);
        }

        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(UrlAdapterError::CredentialsInUrl);
        }

        let host = parsed.host_str().ok_or(UrlAdapterError::MissingHost)?;
        let origin = Origin::new(scheme, host, parsed.port()).map_err(UrlAdapterError::Origin)?;

        Ok(Self { parsed, origin })
    }

    /// Interpreta texto digitado na omnibox sem relaxar os esquemas aceitos.
    ///
    /// Domínios simples recebem HTTPS. Hosts de desenvolvimento locais recebem
    /// HTTP para manter o fluxo de localhost útil no MVP; a política ainda
    /// decide se a navegação pode prosseguir.
    pub fn parse_user_input(input: &str) -> Result<Self, UrlAdapterError> {
        Self::parse(&normalize_user_input(input))
    }

    pub fn as_str(&self) -> &str {
        self.parsed.as_str()
    }

    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    pub fn is_secure(&self) -> bool {
        self.origin.scheme() == Scheme::Https
    }

    pub fn is_loopback(&self) -> bool {
        match self.parsed.host() {
            Some(url::Host::Domain(domain)) => {
                domain.eq_ignore_ascii_case("localhost")
                    || domain.to_ascii_lowercase().ends_with(".localhost")
            }
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        }
    }
}

pub fn normalize_user_input(input: &str) -> String {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();

    if trimmed.is_empty()
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("javascript:")
        || lower.starts_with("file:")
        || lower.starts_with("data:")
        || lower.starts_with("about:")
        || lower.starts_with("view-source:")
    {
        return trimmed.to_owned();
    }

    let host_part = trimmed.split('/').next().unwrap_or(trimmed);
    if is_loopback_host_part(host_part) {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    }
}

fn is_loopback_host_part(host_part: &str) -> bool {
    let lower = host_part.to_ascii_lowercase();
    lower == "localhost"
        || lower.starts_with("localhost:")
        || lower == "127.0.0.1"
        || lower.starts_with("127.0.0.1:")
        || lower == "[::1]"
        || lower.starts_with("[::1]:")
}

#[derive(Debug)]
pub enum UrlAdapterError {
    Parse(url::ParseError),
    UnsupportedScheme,
    MissingHost,
    CredentialsInUrl,
    Origin(OriginError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parseia_https_e_normaliza_a_origem() {
        let url = WebUrl::parse("https://EXAMPLE.com:443/account?tab=security").unwrap();

        assert!(url.is_secure());
        assert_eq!(url.origin().canonical_string(), "https://example.com");
    }

    #[test]
    fn preserva_porta_nao_padrao_na_origem() {
        let url = WebUrl::parse("http://localhost:8080/").unwrap();

        assert_eq!(url.origin().canonical_string(), "http://localhost:8080");
    }

    #[test]
    fn rejeita_esquemas_com_codigo_ou_arquivos() {
        assert!(matches!(
            WebUrl::parse("javascript:alert(1)"),
            Err(UrlAdapterError::UnsupportedScheme)
        ));
        assert!(matches!(
            WebUrl::parse("file:///etc/passwd"),
            Err(UrlAdapterError::UnsupportedScheme)
        ));
    }

    #[test]
    fn rejeita_url_relativa() {
        assert!(matches!(
            WebUrl::parse("/login"),
            Err(UrlAdapterError::Parse(_))
        ));
    }

    #[test]
    fn rejeita_credenciais_embutidas() {
        assert!(matches!(
            WebUrl::parse("https://alice:secret@example.com/"),
            Err(UrlAdapterError::CredentialsInUrl)
        ));
    }

    #[test]
    fn formata_ipv6_com_colchetes() {
        let url = WebUrl::parse("https://[::1]/").unwrap();

        assert_eq!(url.origin().canonical_string(), "https://[::1]");
    }

    #[test]
    fn identifica_loopback_por_nome_e_endereco() {
        assert!(WebUrl::parse("http://localhost:3000/")
            .unwrap()
            .is_loopback());
        assert!(WebUrl::parse("http://127.0.0.1/").unwrap().is_loopback());
        assert!(WebUrl::parse("http://[::1]/").unwrap().is_loopback());
        assert!(!WebUrl::parse("http://example.com/").unwrap().is_loopback());
    }

    #[test]
    fn normaliza_dominio_simples_para_https() {
        assert_eq!(
            normalize_user_input(" example.com/path "),
            "https://example.com/path"
        );
        assert_eq!(
            WebUrl::parse_user_input("example.com").unwrap().as_str(),
            "https://example.com/"
        );
    }

    #[test]
    fn normaliza_loopback_para_http() {
        assert_eq!(
            normalize_user_input("localhost:3000"),
            "http://localhost:3000"
        );
        assert_eq!(
            WebUrl::parse_user_input("127.0.0.1:8080").unwrap().as_str(),
            "http://127.0.0.1:8080/"
        );
    }

    #[test]
    fn nao_esconde_esquema_perigoso() {
        assert_eq!(
            normalize_user_input("javascript:alert(1)"),
            "javascript:alert(1)"
        );
        assert!(WebUrl::parse_user_input("javascript:alert(1)").is_err());
    }
}
