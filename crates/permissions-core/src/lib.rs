//! Políticas conservadoras para APIs poderosas do navegador.

use std::collections::HashMap;

use aegis_policy_core::{Origin, TopLevelSite};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    Camera,
    Microphone,
    Geolocation,
    Notifications,
    ClipboardRead,
    ClipboardWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Ask,
    Granted,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny,
    Prompt,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PermissionKey {
    profile_id: String,
    origin: Origin,
    permission: Permission,
}

impl PermissionKey {
    pub fn new(
        profile_id: &str,
        origin: Origin,
        permission: Permission,
    ) -> Result<Self, PermissionError> {
        if profile_id.is_empty() || profile_id.chars().any(char::is_control) {
            return Err(PermissionError::InvalidProfileId);
        }

        Ok(Self {
            profile_id: profile_id.to_owned(),
            origin,
            permission,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub profile_id: String,
    pub origin: Origin,
    pub top_level_site: TopLevelSite,
    pub permission: Permission,
    pub is_top_level: bool,
    pub is_secure_context: bool,
    pub has_user_gesture: bool,
}

impl PermissionRequest {
    pub fn key(&self) -> Result<PermissionKey, PermissionError> {
        PermissionKey::new(&self.profile_id, self.origin.clone(), self.permission)
    }
}

#[derive(Debug, Default)]
pub struct PermissionStore {
    states: HashMap<PermissionKey, PermissionState>,
}

impl PermissionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, request: &PermissionRequest) -> Result<PermissionState, PermissionError> {
        let key = request.key()?;
        Ok(self
            .states
            .get(&key)
            .copied()
            .unwrap_or(PermissionState::Ask))
    }

    pub fn set(
        &mut self,
        request: &PermissionRequest,
        state: PermissionState,
    ) -> Result<(), PermissionError> {
        let key = request.key()?;
        self.states.insert(key, state);
        Ok(())
    }

    pub fn clear_origin(&mut self, profile_id: &str, origin: &Origin) -> usize {
        let before = self.states.len();
        self.states
            .retain(|key, _| key.profile_id != profile_id || &key.origin != origin);
        before - self.states.len()
    }

    pub fn evaluate(
        &self,
        request: &PermissionRequest,
    ) -> Result<PermissionDecision, PermissionError> {
        let state = self.get(request)?;

        if !context_allows(request) {
            return Ok(PermissionDecision::Deny);
        }

        Ok(match state {
            PermissionState::Granted => PermissionDecision::Allow,
            PermissionState::Denied => PermissionDecision::Deny,
            PermissionState::Ask => PermissionDecision::Prompt,
        })
    }
}

fn context_allows(request: &PermissionRequest) -> bool {
    if !request.is_secure_context || !request.is_top_level || !request.has_user_gesture {
        return false;
    }

    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionError {
    InvalidProfileId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis_policy_core::{Origin, Scheme, TopLevelSite};

    fn request(profile_id: &str, permission: Permission) -> PermissionRequest {
        PermissionRequest {
            profile_id: profile_id.to_owned(),
            origin: Origin::new(Scheme::Https, "app.example", None).unwrap(),
            top_level_site: TopLevelSite::new(Scheme::Https, "app.example").unwrap(),
            permission,
            is_top_level: true,
            is_secure_context: true,
            has_user_gesture: true,
        }
    }

    #[test]
    fn estado_padrao_e_prompt() {
        let store = PermissionStore::new();
        assert_eq!(
            store
                .evaluate(&request("default", Permission::Camera))
                .unwrap(),
            PermissionDecision::Prompt
        );
    }

    #[test]
    fn concessao_e_limitada_ao_perfil_e_origem() {
        let mut store = PermissionStore::new();
        let granted = request("default", Permission::Camera);
        let other_profile = request("private", Permission::Camera);

        store.set(&granted, PermissionState::Granted).unwrap();

        assert_eq!(store.evaluate(&granted).unwrap(), PermissionDecision::Allow);
        assert_eq!(
            store.evaluate(&other_profile).unwrap(),
            PermissionDecision::Prompt
        );
    }

    #[test]
    fn contexto_inseguro_e_negado_mesmo_com_concessao() {
        let mut store = PermissionStore::new();
        let mut insecure = request("default", Permission::Microphone);
        insecure.is_secure_context = false;
        store.set(&insecure, PermissionState::Granted).unwrap();

        assert_eq!(store.evaluate(&insecure).unwrap(), PermissionDecision::Deny);
    }

    #[test]
    fn iframe_nao_recebe_permissao_sensivel() {
        let mut store = PermissionStore::new();
        let mut embedded = request("default", Permission::Geolocation);
        embedded.is_top_level = false;
        store.set(&embedded, PermissionState::Granted).unwrap();

        assert_eq!(store.evaluate(&embedded).unwrap(), PermissionDecision::Deny);
    }

    #[test]
    fn gesto_do_usuario_e_obrigatorio() {
        let mut store = PermissionStore::new();
        let mut automatic = request("default", Permission::Notifications);
        automatic.has_user_gesture = false;
        store.set(&automatic, PermissionState::Granted).unwrap();

        assert_eq!(
            store.evaluate(&automatic).unwrap(),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn negar_persiste_ate_ser_alterado() {
        let mut store = PermissionStore::new();
        let denied = request("default", Permission::ClipboardRead);
        store.set(&denied, PermissionState::Denied).unwrap();

        assert_eq!(store.evaluate(&denied).unwrap(), PermissionDecision::Deny);
    }

    #[test]
    fn limpar_origem_remove_todas_as_permissoes_dela() {
        let mut store = PermissionStore::new();
        let camera = request("default", Permission::Camera);
        let microphone = request("default", Permission::Microphone);
        let origin = camera.origin.clone();

        store.set(&camera, PermissionState::Granted).unwrap();
        store.set(&microphone, PermissionState::Denied).unwrap();

        assert_eq!(store.clear_origin("default", &origin), 2);
        assert_eq!(store.evaluate(&camera).unwrap(), PermissionDecision::Prompt);
    }
}
