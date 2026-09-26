//! Modelo de armazenamento isolado por perfil, site superior e origem.
//!
//! Este crate é deliberadamente agnóstico de disco. Ele define a semântica de
//! isolamento que uma implementação persistente deverá preservar.

use std::collections::HashMap;

use aegis_policy_core::StoragePartitionKey;

/// Áreas que podem conter estado reutilizável por uma página.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageArea {
    Cookies,
    LocalStorage,
    IndexedDb,
    Cache,
    ServiceWorkers,
    HttpCache,
}

/// Armazenamento em memória com isolamento obrigatório por partição.
#[derive(Debug)]
pub struct PartitionedStore<V> {
    entries: HashMap<(StorageArea, StoragePartitionKey, String), V>,
}

impl<V> Default for PartitionedStore<V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<V> PartitionedStore<V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(
        &mut self,
        area: StorageArea,
        partition: StoragePartitionKey,
        name: &str,
        value: V,
    ) -> Result<(), StorageError> {
        validate_name(name)?;
        self.entries
            .insert((area, partition, name.to_owned()), value);
        Ok(())
    }

    pub fn get(
        &self,
        area: StorageArea,
        partition: &StoragePartitionKey,
        name: &str,
    ) -> Result<Option<&V>, StorageError> {
        validate_name(name)?;
        Ok(self
            .entries
            .get(&(area, partition.clone(), name.to_owned())))
    }

    pub fn remove(
        &mut self,
        area: StorageArea,
        partition: &StoragePartitionKey,
        name: &str,
    ) -> Result<Option<V>, StorageError> {
        validate_name(name)?;
        Ok(self
            .entries
            .remove(&(area, partition.clone(), name.to_owned())))
    }

    /// Remove tudo de uma partição, opcionalmente limitado a uma área.
    pub fn clear_partition(
        &mut self,
        area: Option<StorageArea>,
        partition: &StoragePartitionKey,
    ) -> usize {
        let before = self.entries.len();
        self.entries.retain(|(entry_area, entry_partition, _), _| {
            let same_partition = entry_partition == partition;
            let same_area = area.is_none_or(|requested| requested == *entry_area);
            !(same_partition && same_area)
        });
        before - self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn validate_name(name: &str) -> Result<(), StorageError> {
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(StorageError::InvalidName);
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    InvalidName,
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis_policy_core::{Origin, Scheme, StoragePartitionKey, TopLevelSite};

    fn partition(profile: &str, site: &str) -> StoragePartitionKey {
        let resource = Origin::new(Scheme::Https, "tracker.example", None).unwrap();
        let top_level = TopLevelSite::new(Scheme::Https, site).unwrap();
        StoragePartitionKey::new(profile, top_level, resource).unwrap()
    }

    #[test]
    fn estado_nao_vaza_entre_sites_superiores() {
        let mut store = PartitionedStore::new();
        let news = partition("default", "news.example");
        let shop = partition("default", "shop.example");

        store
            .set(StorageArea::Cookies, news.clone(), "session", "news-user")
            .unwrap();

        assert_eq!(
            store.get(StorageArea::Cookies, &news, "session").unwrap(),
            Some(&"news-user")
        );
        assert_eq!(
            store.get(StorageArea::Cookies, &shop, "session").unwrap(),
            None
        );
    }

    #[test]
    fn areas_diferentes_nao_compartilham_estado() {
        let mut store = PartitionedStore::new();
        let key = partition("default", "news.example");

        store
            .set(StorageArea::Cookies, key.clone(), "token", "cookie")
            .unwrap();

        assert_eq!(
            store.get(StorageArea::LocalStorage, &key, "token").unwrap(),
            None
        );
    }

    #[test]
    fn limpar_uma_area_preserva_as_outras() {
        let mut store = PartitionedStore::new();
        let key = partition("default", "news.example");

        store
            .set(StorageArea::Cookies, key.clone(), "cookie", "value")
            .unwrap();
        store
            .set(StorageArea::Cache, key.clone(), "page", "cached")
            .unwrap();

        assert_eq!(store.clear_partition(Some(StorageArea::Cookies), &key), 1);
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.get(StorageArea::Cache, &key, "page").unwrap(),
            Some(&"cached")
        );
    }

    #[test]
    fn rejeita_nome_vazio_ou_controle() {
        let mut store = PartitionedStore::new();
        let key = partition("default", "news.example");

        assert_eq!(
            store.set(StorageArea::Cookies, key.clone(), "", "value"),
            Err(StorageError::InvalidName)
        );
        assert_eq!(
            store.get(StorageArea::Cookies, &key, "bad\nname"),
            Err(StorageError::InvalidName)
        );
    }
}
