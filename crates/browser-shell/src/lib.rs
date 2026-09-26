//! Estado do shell do navegador, sem dependência de uma GUI específica.
//!
//! Este crate coordena abas e navegação, mas mantém decisões de segurança nos
//! crates de política, URL e permissões.

use std::collections::HashMap;

use aegis_permissions_core::{
    PermissionDecision, PermissionError, PermissionRequest, PermissionState, PermissionStore,
};
use aegis_policy_core::{BrowserPolicy, NavigationDecision, NavigationRequest};
use aegis_url_adapter::{UrlAdapterError, WebUrl};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

impl TabId {
    pub fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct Tab {
    id: TabId,
    current_url: Option<WebUrl>,
    status: TabStatus,
    title: Option<String>,
    progress: u8,
    error: Option<LoadError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabStatus {
    Blank,
    Loading,
    Loaded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    Network,
    Certificate,
    RendererCrashed,
    Cancelled,
    Unknown,
}

impl Tab {
    pub fn id(&self) -> TabId {
        self.id
    }

    pub fn current_url(&self) -> Option<&WebUrl> {
        self.current_url.as_ref()
    }

    pub fn status(&self) -> TabStatus {
        self.status
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn progress(&self) -> u8 {
        self.progress
    }

    pub fn error(&self) -> Option<LoadError> {
        self.error
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationResult {
    Navigated { tab_id: TabId, url: String },
    ConfirmationRequired { tab_id: TabId, url: String },
    BlockedInsecure { tab_id: TabId, url: String },
}

/// Fronteira mínima entre o shell privilegiado e uma engine de renderização.
///
/// A implementação concreta poderá usar Chromium, Gecko ou outra engine. Ela
/// recebe apenas URLs que já passaram pelo `BrowserPolicy`.
pub trait NavigationEngine {
    fn load(&mut self, tab_id: TabId, url: &WebUrl) -> Result<(), EngineError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    Unavailable,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineEvent {
    LoadStarted { tab_id: TabId },
    Progress { tab_id: TabId, value: u8 },
    LoadFinished { tab_id: TabId },
    LoadFailed { tab_id: TabId, error: LoadError },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectDecision {
    Allow { url: String },
    BlockInsecure { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleChanged {
    pub tab_id: TabId,
    pub title: String,
}

#[derive(Debug)]
pub enum ShellError {
    UnknownTab(TabId),
    NoPendingNavigation(TabId),
    InvalidUrl(UrlAdapterError),
    Permission(PermissionError),
    Engine(EngineError),
    InvalidEngineEvent(EngineEventError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineEventError {
    InvalidProgress(u8),
}

#[derive(Debug, Default)]
pub struct BrowserShell {
    tabs: HashMap<TabId, Tab>,
    active_tab: Option<TabId>,
    pending_insecure: HashMap<TabId, WebUrl>,
    next_tab_id: u64,
    policy: BrowserPolicy,
    permissions: PermissionStore,
}

impl BrowserShell {
    pub fn new(policy: BrowserPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    pub fn new_tab(&mut self) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        self.tabs.insert(
            id,
            Tab {
                id,
                current_url: None,
                status: TabStatus::Blank,
                title: None,
                progress: 0,
                error: None,
            },
        );
        self.active_tab = Some(id);
        id
    }

    pub fn close_tab(&mut self, tab_id: TabId) -> Result<(), ShellError> {
        if self.tabs.remove(&tab_id).is_none() {
            return Err(ShellError::UnknownTab(tab_id));
        }

        self.pending_insecure.remove(&tab_id);
        if self.active_tab == Some(tab_id) {
            self.active_tab = self.tabs.keys().next().copied();
        }
        Ok(())
    }

    pub fn tab(&self, tab_id: TabId) -> Result<&Tab, ShellError> {
        self.tabs.get(&tab_id).ok_or(ShellError::UnknownTab(tab_id))
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active_tab.and_then(|tab_id| self.tabs.get(&tab_id))
    }

    pub fn active_tab_id(&self) -> Option<TabId> {
        self.active_tab
    }

    pub fn tab_ids(&self) -> Vec<TabId> {
        let mut ids: Vec<_> = self.tabs.keys().copied().collect();
        ids.sort_by_key(|tab_id| tab_id.0);
        ids
    }

    pub fn select_tab(&mut self, tab_id: TabId) -> Result<(), ShellError> {
        self.tab(tab_id)?;
        self.active_tab = Some(tab_id);
        Ok(())
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Retorna a política efetiva usada por novas navegações.
    pub fn policy(&self) -> BrowserPolicy {
        self.policy
    }

    /// Atualiza a política sem permitir que uma confirmação HTTP antiga
    /// sobreviva a uma mudança feita pelo usuário nas configurações.
    pub fn set_policy(&mut self, policy: BrowserPolicy) {
        self.policy = policy;
        self.pending_insecure.clear();
    }

    pub fn navigate(
        &mut self,
        tab_id: TabId,
        input: &str,
        has_user_gesture: bool,
    ) -> Result<NavigationResult, ShellError> {
        let mut accepting_engine = AcceptingEngine;
        self.navigate_with_engine(&mut accepting_engine, tab_id, input, has_user_gesture)
    }

    pub fn navigate_with_engine<E: NavigationEngine>(
        &mut self,
        engine: &mut E,
        tab_id: TabId,
        input: &str,
        has_user_gesture: bool,
    ) -> Result<NavigationResult, ShellError> {
        self.tab(tab_id)?;
        let url = WebUrl::parse_user_input(input).map_err(ShellError::InvalidUrl)?;
        let decision = self.policy.evaluate_navigation(NavigationRequest {
            scheme: url.origin().scheme(),
            is_loopback: url.is_loopback(),
            is_top_level: true,
            has_user_gesture,
        });

        let result = match decision {
            NavigationDecision::Allow => {
                engine.load(tab_id, &url).map_err(ShellError::Engine)?;
                self.commit_navigation(tab_id, url.clone());
                NavigationResult::Navigated {
                    tab_id,
                    url: url.as_str().to_owned(),
                }
            }
            NavigationDecision::ConfirmInsecure => {
                self.pending_insecure.insert(tab_id, url.clone());
                NavigationResult::ConfirmationRequired {
                    tab_id,
                    url: url.as_str().to_owned(),
                }
            }
            NavigationDecision::BlockInsecure => NavigationResult::BlockedInsecure {
                tab_id,
                url: url.as_str().to_owned(),
            },
            NavigationDecision::IgnoreForSubresource => {
                unreachable!("top-level navigation must not produce a subresource decision")
            }
        };

        Ok(result)
    }

    pub fn confirm_insecure_navigation(
        &mut self,
        tab_id: TabId,
    ) -> Result<NavigationResult, ShellError> {
        let mut accepting_engine = AcceptingEngine;
        self.confirm_insecure_navigation_with_engine(&mut accepting_engine, tab_id)
    }

    pub fn confirm_insecure_navigation_with_engine<E: NavigationEngine>(
        &mut self,
        engine: &mut E,
        tab_id: TabId,
    ) -> Result<NavigationResult, ShellError> {
        self.tab(tab_id)?;
        let url = self
            .pending_insecure
            .remove(&tab_id)
            .ok_or(ShellError::NoPendingNavigation(tab_id))?;
        engine.load(tab_id, &url).map_err(ShellError::Engine)?;
        self.commit_navigation(tab_id, url.clone());
        Ok(NavigationResult::Navigated {
            tab_id,
            url: url.as_str().to_owned(),
        })
    }

    pub fn cancel_insecure_navigation(&mut self, tab_id: TabId) -> Result<(), ShellError> {
        self.tab(tab_id)?;
        self.pending_insecure.remove(&tab_id);
        Ok(())
    }

    pub fn handle_engine_event(&mut self, event: EngineEvent) -> Result<(), ShellError> {
        let tab_id = match event {
            EngineEvent::LoadStarted { tab_id }
            | EngineEvent::Progress { tab_id, .. }
            | EngineEvent::LoadFinished { tab_id }
            | EngineEvent::LoadFailed { tab_id, .. } => tab_id,
        };
        self.tab(tab_id)?;

        match event {
            EngineEvent::LoadStarted { .. } => {
                let tab = self.tabs.get_mut(&tab_id).expect("tab validated above");
                tab.status = TabStatus::Loading;
                tab.progress = 0;
                tab.error = None;
            }
            EngineEvent::Progress { value, .. } => {
                if value > 100 {
                    return Err(ShellError::InvalidEngineEvent(
                        EngineEventError::InvalidProgress(value),
                    ));
                }
                let tab = self.tabs.get_mut(&tab_id).expect("tab validated above");
                tab.status = TabStatus::Loading;
                tab.progress = value;
            }
            EngineEvent::LoadFinished { .. } => {
                let tab = self.tabs.get_mut(&tab_id).expect("tab validated above");
                tab.status = TabStatus::Loaded;
                tab.progress = 100;
                tab.error = None;
            }
            EngineEvent::LoadFailed { error, .. } => {
                let tab = self.tabs.get_mut(&tab_id).expect("tab validated above");
                tab.status = TabStatus::Failed;
                tab.error = Some(error);
            }
        }

        Ok(())
    }

    pub fn handle_title_changed(&mut self, event: TitleChanged) -> Result<(), ShellError> {
        let tab = self
            .tabs
            .get_mut(&event.tab_id)
            .ok_or(ShellError::UnknownTab(event.tab_id))?;
        tab.title = Some(sanitize_title(&event.title));
        Ok(())
    }

    /// Valida um redirect recebido da engine.
    ///
    /// Redirects não carregam gesto novo do usuário. Por isso, HTTP não pode
    /// virar uma exceção apenas porque uma página HTTPS o solicitou.
    pub fn evaluate_engine_redirect(
        &self,
        tab_id: TabId,
        input: &str,
    ) -> Result<RedirectDecision, ShellError> {
        self.tab(tab_id)?;
        let url = WebUrl::parse(input).map_err(ShellError::InvalidUrl)?;
        let decision = self.policy.evaluate_navigation(NavigationRequest {
            scheme: url.origin().scheme(),
            is_loopback: url.is_loopback(),
            is_top_level: true,
            has_user_gesture: false,
        });

        match decision {
            NavigationDecision::Allow => Ok(RedirectDecision::Allow {
                url: url.as_str().to_owned(),
            }),
            NavigationDecision::BlockInsecure | NavigationDecision::ConfirmInsecure => {
                Ok(RedirectDecision::BlockInsecure {
                    url: url.as_str().to_owned(),
                })
            }
            NavigationDecision::IgnoreForSubresource => {
                unreachable!("top-level redirect must not produce a subresource decision")
            }
        }
    }

    pub fn evaluate_permission(
        &self,
        request: &PermissionRequest,
    ) -> Result<PermissionDecision, ShellError> {
        self.permissions
            .evaluate(request)
            .map_err(ShellError::Permission)
    }

    pub fn set_permission(
        &mut self,
        request: &PermissionRequest,
        state: PermissionState,
    ) -> Result<(), ShellError> {
        self.permissions
            .set(request, state)
            .map_err(ShellError::Permission)
    }

    fn commit_navigation(&mut self, tab_id: TabId, url: WebUrl) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.current_url = Some(url);
            tab.status = TabStatus::Loading;
            tab.progress = 0;
            tab.error = None;
        }
        self.active_tab = Some(tab_id);
    }
}

fn sanitize_title(title: &str) -> String {
    title
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect()
}

struct AcceptingEngine;

impl NavigationEngine for AcceptingEngine {
    fn load(&mut self, _tab_id: TabId, _url: &WebUrl) -> Result<(), EngineError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestEngine {
        loads: Vec<(TabId, String)>,
        fail: bool,
    }

    impl NavigationEngine for TestEngine {
        fn load(&mut self, tab_id: TabId, url: &WebUrl) -> Result<(), EngineError> {
            if self.fail {
                return Err(EngineError::Rejected);
            }
            self.loads.push((tab_id, url.as_str().to_owned()));
            Ok(())
        }
    }

    #[test]
    fn cria_aba_ativa() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();

        assert_eq!(id.value(), 0);
        assert_eq!(shell.tab_count(), 1);
        assert_eq!(shell.active_tab().unwrap().id(), id);
        assert!(shell.active_tab().unwrap().current_url().is_none());
        assert_eq!(shell.active_tab().unwrap().status(), TabStatus::Blank);
        assert_eq!(shell.active_tab_id(), Some(id));
    }

    #[test]
    fn seleciona_aba_existente() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let first = shell.new_tab();
        let second = shell.new_tab();

        shell.select_tab(first).unwrap();

        assert_eq!(shell.active_tab_id(), Some(first));
        assert_eq!(shell.tab_ids(), vec![first, second]);
    }

    #[test]
    fn navega_https_e_atualiza_a_aba() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();

        let result = shell.navigate(id, "https://example.com/", false).unwrap();

        assert_eq!(
            result,
            NavigationResult::Navigated {
                tab_id: id,
                url: "https://example.com/".to_owned()
            }
        );
        assert_eq!(
            shell.tab(id).unwrap().current_url().unwrap().as_str(),
            "https://example.com/"
        );
        assert_eq!(shell.tab(id).unwrap().status(), TabStatus::Loading);
    }

    #[test]
    fn bloqueia_http_sem_gesto() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();

        assert!(matches!(
            shell.navigate(id, "http://example.com/", false),
            Ok(NavigationResult::BlockedInsecure { .. })
        ));
        assert!(shell.tab(id).unwrap().current_url().is_none());
    }

    #[test]
    fn confirma_http_com_gesto_antes_de_navegar() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();

        assert!(matches!(
            shell.navigate(id, "http://example.com/", true),
            Ok(NavigationResult::ConfirmationRequired { .. })
        ));
        assert!(shell.tab(id).unwrap().current_url().is_none());

        shell.confirm_insecure_navigation(id).unwrap();
        assert_eq!(
            shell.tab(id).unwrap().current_url().unwrap().as_str(),
            "http://example.com/"
        );
    }

    #[test]
    fn localhost_http_e_permitido_no_mvp() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();

        assert!(matches!(
            shell.navigate(id, "http://localhost:3000/", false),
            Ok(NavigationResult::Navigated { .. })
        ));
    }

    #[test]
    fn atualiza_politica_e_invalida_confirmacoes_pendentes() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();

        assert!(matches!(
            shell.navigate(id, "http://example.com", true),
            Ok(NavigationResult::ConfirmationRequired { .. })
        ));

        let mut policy = shell.policy();
        policy.https_only = false;
        shell.set_policy(policy);

        assert_eq!(shell.policy(), policy);
        assert!(matches!(
            shell.confirm_insecure_navigation(id),
            Err(ShellError::NoPendingNavigation(_))
        ));
    }

    #[test]
    fn url_invalida_nao_altera_a_aba() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();

        assert!(matches!(
            shell.navigate(id, "javascript:alert(1)", true),
            Err(ShellError::InvalidUrl(_))
        ));
        assert!(shell.tab(id).unwrap().current_url().is_none());
    }

    #[test]
    fn fechar_aba_remove_confirmacao_pendente() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();
        shell.navigate(id, "http://example.com/", true).unwrap();
        shell.close_tab(id).unwrap();

        assert!(matches!(
            shell.confirm_insecure_navigation(id),
            Err(ShellError::UnknownTab(_))
        ));
    }

    #[test]
    fn confirmar_sem_pendente_retorna_erro_especifico() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();

        assert!(matches!(
            shell.confirm_insecure_navigation(id),
            Err(ShellError::NoPendingNavigation(_))
        ));
    }

    #[test]
    fn engine_recebe_apenas_navegacao_aprovada() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();
        let mut engine = TestEngine::default();

        shell
            .navigate_with_engine(&mut engine, id, "http://example.com/", false)
            .unwrap();

        assert!(engine.loads.is_empty());
    }

    #[test]
    fn falha_da_engine_nao_atualiza_a_aba() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();
        let mut engine = TestEngine {
            fail: true,
            ..TestEngine::default()
        };

        assert!(matches!(
            shell.navigate_with_engine(&mut engine, id, "https://example.com/", false),
            Err(ShellError::Engine(EngineError::Rejected))
        ));
        assert!(shell.tab(id).unwrap().current_url().is_none());
    }

    #[test]
    fn engine_recebe_https_antes_do_commit_da_aba() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();
        let mut engine = TestEngine::default();

        shell
            .navigate_with_engine(&mut engine, id, "https://example.com/", false)
            .unwrap();

        assert_eq!(engine.loads, vec![(id, "https://example.com/".to_owned())]);
        assert!(shell.tab(id).unwrap().current_url().is_some());
    }

    #[test]
    fn eventos_atualizam_estado_de_carregamento() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();
        shell.navigate(id, "https://example.com/", false).unwrap();

        shell
            .handle_engine_event(EngineEvent::Progress {
                tab_id: id,
                value: 50,
            })
            .unwrap();
        assert_eq!(shell.tab(id).unwrap().progress(), 50);
        assert_eq!(shell.tab(id).unwrap().status(), TabStatus::Loading);

        shell
            .handle_engine_event(EngineEvent::LoadFinished { tab_id: id })
            .unwrap();
        assert_eq!(shell.tab(id).unwrap().progress(), 100);
        assert_eq!(shell.tab(id).unwrap().status(), TabStatus::Loaded);
    }

    #[test]
    fn falha_de_carregamento_fica_visivel_na_aba() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();
        shell.navigate(id, "https://example.com/", false).unwrap();

        shell
            .handle_engine_event(EngineEvent::LoadFailed {
                tab_id: id,
                error: LoadError::Certificate,
            })
            .unwrap();

        assert_eq!(shell.tab(id).unwrap().status(), TabStatus::Failed);
        assert_eq!(shell.tab(id).unwrap().error(), Some(LoadError::Certificate));
    }

    #[test]
    fn progresso_invalido_e_rejeitado() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();

        assert!(matches!(
            shell.handle_engine_event(EngineEvent::Progress {
                tab_id: id,
                value: 101,
            }),
            Err(ShellError::InvalidEngineEvent(
                EngineEventError::InvalidProgress(101)
            ))
        ));
    }

    #[test]
    fn titulo_e_sanitizado_e_limitado() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();
        let title = format!("A\nB{}", "x".repeat(600));

        shell
            .handle_title_changed(TitleChanged { tab_id: id, title })
            .unwrap();

        let stored = shell.tab(id).unwrap().title().unwrap();
        assert_eq!(stored, format!("AB{}", "x".repeat(510)));
    }

    #[test]
    fn redirect_https_para_http_e_bloqueado() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();
        shell.navigate(id, "https://example.com/", false).unwrap();

        assert_eq!(
            shell
                .evaluate_engine_redirect(id, "http://example.com/login")
                .unwrap(),
            RedirectDecision::BlockInsecure {
                url: "http://example.com/login".to_owned()
            }
        );
        assert_eq!(
            shell.tab(id).unwrap().current_url().unwrap().as_str(),
            "https://example.com/"
        );
    }

    #[test]
    fn redirect_https_para_https_e_permitido() {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        let id = shell.new_tab();
        shell.navigate(id, "https://example.com/", false).unwrap();

        assert_eq!(
            shell
                .evaluate_engine_redirect(id, "https://example.com/account")
                .unwrap(),
            RedirectDecision::Allow {
                url: "https://example.com/account".to_owned()
            }
        );
    }
}
