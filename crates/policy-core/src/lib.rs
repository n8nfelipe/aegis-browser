//! Núcleo determinístico de políticas do Aegis Browser.
//!
//! Este crate não faz parsing de URLs, acesso à rede, acesso ao disco ou UI.
//! Um adaptador privilegiado deve normalizar a entrada e então consultar este
//! núcleo. Manter essa separação reduz a superfície de ataque e facilita testes.

/// Esquemas aceitos pela camada de decisão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    Http,
    Https,
    Other,
}

/// Uma origem web já normalizada por um parser confiável.
///
/// Este tipo não recebe uma URL completa de propósito. Parsing de URL, IDNA,
/// IPv6 e Public Suffix List devem ficar em um adaptador especializado. O
/// núcleo de políticas só aceita os campos normalizados.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin {
    scheme: Scheme,
    host: String,
    port: Option<u16>,
}

impl Origin {
    pub fn new(scheme: Scheme, host: &str, port: Option<u16>) -> Result<Self, OriginError> {
        if !matches!(scheme, Scheme::Http | Scheme::Https) {
            return Err(OriginError::UnsupportedScheme);
        }

        if host.is_empty() || host.chars().any(char::is_control) {
            return Err(OriginError::InvalidHost);
        }

        let normalized_host = host.to_ascii_lowercase();
        let normalized_port = match (scheme, port) {
            (Scheme::Http, Some(80)) | (Scheme::Https, Some(443)) => None,
            (_, value) => value,
        };

        Ok(Self {
            scheme,
            host: normalized_host,
            port: normalized_port,
        })
    }

    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn canonical_string(&self) -> String {
        let scheme = match self.scheme {
            Scheme::Http => "http",
            Scheme::Https => "https",
            Scheme::Other => unreachable!("Origin::new rejects other schemes"),
        };

        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };

        match self.port {
            Some(port) => format!("{scheme}://{host}:{port}"),
            None => format!("{scheme}://{host}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginError {
    UnsupportedScheme,
    InvalidHost,
}

/// Identidade do site superior, calculada pelo adaptador usando uma Public
/// Suffix List atualizada. Não tente derivar este valor com `host.split('.')`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TopLevelSite {
    scheme: Scheme,
    registrable_domain: String,
}

impl TopLevelSite {
    pub fn new(scheme: Scheme, registrable_domain: &str) -> Result<Self, OriginError> {
        if !matches!(scheme, Scheme::Http | Scheme::Https) {
            return Err(OriginError::UnsupportedScheme);
        }

        if registrable_domain.is_empty() || registrable_domain.chars().any(char::is_control) {
            return Err(OriginError::InvalidHost);
        }

        Ok(Self {
            scheme,
            registrable_domain: registrable_domain.to_ascii_lowercase(),
        })
    }

    pub fn canonical_string(&self) -> String {
        let scheme = match self.scheme {
            Scheme::Http => "http",
            Scheme::Https => "https",
            Scheme::Other => unreachable!("TopLevelSite::new rejects other schemes"),
        };

        format!("{scheme}://{}", self.registrable_domain)
    }
}

/// Chave double-keyed para estado de terceiros.
///
/// O perfil também participa da chave para impedir que perfis independentes
/// compartilhem cookies, cache ou identificadores por acidente.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoragePartitionKey {
    profile_id: String,
    top_level_site: TopLevelSite,
    resource_origin: Origin,
}

impl StoragePartitionKey {
    pub fn new(
        profile_id: &str,
        top_level_site: TopLevelSite,
        resource_origin: Origin,
    ) -> Result<Self, StoragePartitionError> {
        if profile_id.is_empty() || profile_id.chars().any(char::is_control) {
            return Err(StoragePartitionError::InvalidProfileId);
        }

        Ok(Self {
            profile_id: profile_id.to_owned(),
            top_level_site,
            resource_origin,
        })
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    pub fn top_level_site(&self) -> &TopLevelSite {
        &self.top_level_site
    }

    pub fn resource_origin(&self) -> &Origin {
        &self.resource_origin
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoragePartitionError {
    InvalidProfileId,
}

/// Entrada já normalizada pelo componente que entende URLs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationRequest {
    pub scheme: Scheme,
    pub is_loopback: bool,
    pub is_top_level: bool,
    pub has_user_gesture: bool,
}

/// Resultado explícito de uma decisão de navegação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationDecision {
    Allow,
    BlockInsecure,
    ConfirmInsecure,
    IgnoreForSubresource,
}

/// Configuração mínima e deliberadamente pequena do MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserPolicy {
    pub https_only: bool,
    pub allow_loopback_http: bool,
    pub confirm_http_exceptions: bool,
}

impl Default for BrowserPolicy {
    fn default() -> Self {
        Self {
            https_only: true,
            allow_loopback_http: true,
            confirm_http_exceptions: true,
        }
    }
}

impl BrowserPolicy {
    /// Decide se uma navegação pode prosseguir.
    ///
    /// A política trata apenas navegações. Redirects e sub-recursos devem ser
    /// reavaliados pelo chamador com o contexto atualizado.
    pub fn evaluate_navigation(&self, request: NavigationRequest) -> NavigationDecision {
        if request.scheme != Scheme::Http || !self.https_only {
            return NavigationDecision::Allow;
        }

        if !request.is_top_level {
            return NavigationDecision::IgnoreForSubresource;
        }

        if request.is_loopback && self.allow_loopback_http {
            return NavigationDecision::Allow;
        }

        if self.confirm_http_exceptions && request.has_user_gesture {
            NavigationDecision::ConfirmInsecure
        } else {
            NavigationDecision::BlockInsecure
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOP_LEVEL: bool = true;

    #[test]
    fn permite_https() {
        let request = NavigationRequest {
            scheme: Scheme::Https,
            is_loopback: false,
            is_top_level: TOP_LEVEL,
            has_user_gesture: false,
        };

        assert_eq!(
            BrowserPolicy::default().evaluate_navigation(request),
            NavigationDecision::Allow
        );
    }

    #[test]
    fn bloque_http_sem_interacao() {
        let request = NavigationRequest {
            scheme: Scheme::Http,
            is_loopback: false,
            is_top_level: TOP_LEVEL,
            has_user_gesture: false,
        };

        assert_eq!(
            BrowserPolicy::default().evaluate_navigation(request),
            NavigationDecision::BlockInsecure
        );
    }

    #[test]
    fn pede_confirmacao_para_excecao_http_iniciada_pelo_usuario() {
        let request = NavigationRequest {
            scheme: Scheme::Http,
            is_loopback: false,
            is_top_level: TOP_LEVEL,
            has_user_gesture: true,
        };

        assert_eq!(
            BrowserPolicy::default().evaluate_navigation(request),
            NavigationDecision::ConfirmInsecure
        );
    }

    #[test]
    fn permite_http_localhost_no_mvp() {
        let request = NavigationRequest {
            scheme: Scheme::Http,
            is_loopback: true,
            is_top_level: TOP_LEVEL,
            has_user_gesture: false,
        };

        assert_eq!(
            BrowserPolicy::default().evaluate_navigation(request),
            NavigationDecision::Allow
        );
    }

    #[test]
    fn nao_bloqueia_subrecurso_neste_nucleo() {
        let request = NavigationRequest {
            scheme: Scheme::Http,
            is_loopback: false,
            is_top_level: false,
            has_user_gesture: false,
        };

        assert_eq!(
            BrowserPolicy::default().evaluate_navigation(request),
            NavigationDecision::IgnoreForSubresource
        );
    }

    #[test]
    fn normaliza_host_e_porta_padrao_da_origem() {
        let origin = Origin::new(Scheme::Https, "Example.COM", Some(443)).unwrap();

        assert_eq!(origin.host(), "example.com");
        assert_eq!(origin.port(), None);
        assert_eq!(origin.canonical_string(), "https://example.com");
    }

    #[test]
    fn preserva_porta_nao_padrao() {
        let origin = Origin::new(Scheme::Https, "example.com", Some(8443)).unwrap();

        assert_eq!(origin.canonical_string(), "https://example.com:8443");
    }

    #[test]
    fn rejeita_esquema_ou_host_invalido() {
        assert_eq!(
            Origin::new(Scheme::Other, "example.com", None),
            Err(OriginError::UnsupportedScheme)
        );
        assert_eq!(
            Origin::new(Scheme::Https, "", None),
            Err(OriginError::InvalidHost)
        );
    }

    #[test]
    fn particao_muda_com_o_site_superior() {
        let origin = Origin::new(Scheme::Https, "tracker.example", None).unwrap();
        let news = TopLevelSite::new(Scheme::Https, "news.example").unwrap();
        let shop = TopLevelSite::new(Scheme::Https, "shop.example").unwrap();

        let news_key = StoragePartitionKey::new("default", news, origin.clone()).unwrap();
        let shop_key = StoragePartitionKey::new("default", shop, origin).unwrap();

        assert_ne!(news_key, shop_key);
    }

    #[test]
    fn particao_muda_com_o_perfil() {
        let origin = Origin::new(Scheme::Https, "tracker.example", None).unwrap();
        let site = TopLevelSite::new(Scheme::Https, "news.example").unwrap();

        let regular = StoragePartitionKey::new("regular", site.clone(), origin.clone()).unwrap();
        let private = StoragePartitionKey::new("private", site, origin).unwrap();

        assert_ne!(regular, private);
    }
}
