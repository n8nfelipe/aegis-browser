use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;

use aegis_browser_shell::{
    BrowserShell, EngineEvent, LoadError, NavigationEngine, NavigationResult, ShellError, TabId,
    TitleChanged,
};
use aegis_policy_core::{BrowserPolicy, NavigationDecision, NavigationRequest};
use aegis_url_adapter::WebUrl;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, ButtonsType, CheckButton, ComboBoxText,
    Dialog, DialogFlags, Entry, FileChooserAction, FileChooserDialog, FileFilter, Grid, IconSize,
    Image, Label, ListBox, ListBoxRow, MessageDialog, MessageType, Notebook, Orientation,
    PositionType, ProgressBar, ResponseType, ScrolledWindow, Window, WindowType,
};
use serde::{Deserialize, Serialize};
use webkit2gtk::{
    CacheModel, CookieAcceptPolicy, CookieManagerExt, CookiePersistentStorage, DownloadExt,
    HardwareAccelerationPolicy, LoadEvent, NavigationPolicyDecision, NavigationPolicyDecisionExt,
    PolicyDecisionExt, PolicyDecisionType, Settings, SettingsExt, URIRequestExt, URIResponseExt,
    WebContext, WebContextExt, WebView, WebViewExt, WebsiteDataManager, WebsiteDataManagerExt,
};

// Gmail rejects the stock WebKitGTK/Safari identity before it even evaluates
// the page. Keep the engine unchanged, but expose a Chromium-compatible UA so
// supported web apps do not classify Aegis as an obsolete embedded browser.
pub(crate) const COMPATIBLE_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36";

struct GtkEngine {
    webview: WebView,
    approved_loads: Rc<RefCell<HashSet<String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemePreference {
    System,
    Light,
    Dark,
}

impl ThemePreference {
    fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::System => "Seguir o sistema",
            Self::Light => "Claro",
            Self::Dark => "Escuro",
        }
    }

    fn from_id(id: Option<glib::GString>) -> Self {
        Self::from_identifier(id.as_deref())
    }

    fn from_identifier(id: Option<&str>) -> Self {
        match id {
            Some("light") => Self::Light,
            Some("dark") => Self::Dark,
            _ => Self::System,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchEngine {
    DuckDuckGo,
    Brave,
    Startpage,
    Qwant,
    Bing,
    Google,
}

impl SearchEngine {
    fn id(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "duckduckgo",
            Self::Brave => "brave",
            Self::Startpage => "startpage",
            Self::Qwant => "qwant",
            Self::Bing => "bing",
            Self::Google => "google",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Brave => "Brave Search",
            Self::Startpage => "Startpage",
            Self::Qwant => "Qwant",
            Self::Bing => "Bing",
            Self::Google => "Google",
        }
    }

    fn from_id(id: Option<glib::GString>) -> Self {
        Self::from_identifier(id.as_deref())
    }

    fn from_identifier(id: Option<&str>) -> Self {
        match id {
            Some("brave") => Self::Brave,
            Some("startpage") => Self::Startpage,
            Some("qwant") => Self::Qwant,
            Some("bing") => Self::Bing,
            Some("google") => Self::Google,
            _ => Self::DuckDuckGo,
        }
    }

    pub(crate) fn search_url(self, query: &str) -> String {
        let base = match self {
            Self::DuckDuckGo => "https://duckduckgo.com/?q=",
            Self::Brave => "https://search.brave.com/search?q=",
            Self::Startpage => "https://www.startpage.com/sp/search?query=",
            Self::Qwant => "https://www.qwant.com/?q=",
            Self::Bing => "https://www.bing.com/search?q=",
            Self::Google => "https://www.google.com/search?q=",
        };
        format!("{base}{}", percent_encode_query(query))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BrowserPreferences {
    pub theme: ThemePreference,
    pub search_engine: SearchEngine,
    pub show_bookmarks_bar: bool,
    pub active_profile: String,
    pub profiles: Vec<String>,
    pub https_only: bool,
    pub allow_loopback_http: bool,
    pub confirm_http_exceptions: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredPreferences {
    #[serde(default)]
    search_engine: Option<String>,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    show_bookmarks_bar: Option<bool>,
    #[serde(default)]
    active_profile: Option<String>,
    #[serde(default)]
    profiles: Option<Vec<String>>,
    #[serde(default)]
    https_only: Option<bool>,
    #[serde(default)]
    allow_loopback_http: Option<bool>,
    #[serde(default)]
    confirm_http_exceptions: Option<bool>,
}

fn preferences_directory() -> PathBuf {
    user_data_root().join("aegis-browser")
}

fn preferences_path() -> PathBuf {
    preferences_directory().join("preferences.json")
}

fn search_engine_path() -> PathBuf {
    preferences_directory().join("search-engine.txt")
}

pub(crate) fn load_preferences() -> BrowserPreferences {
    let mut preferences = BrowserPreferences::default();
    let path = preferences_path();
    let stored = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<StoredPreferences>(&contents).ok());

    if let Some(stored) = stored {
        if let Some(search_engine) = stored.search_engine.as_deref() {
            preferences.search_engine = SearchEngine::from_identifier(Some(search_engine));
        }
        if let Some(theme) = stored.theme.as_deref() {
            preferences.theme = ThemePreference::from_identifier(Some(theme));
        }
        if let Some(show_bookmarks_bar) = stored.show_bookmarks_bar {
            preferences.show_bookmarks_bar = show_bookmarks_bar;
        }
        if let Some(https_only) = stored.https_only {
            preferences.https_only = https_only;
        }
        if let Some(allow_loopback_http) = stored.allow_loopback_http {
            preferences.allow_loopback_http = allow_loopback_http;
        }
        if let Some(confirm_http_exceptions) = stored.confirm_http_exceptions {
            preferences.confirm_http_exceptions = confirm_http_exceptions;
        }
        if let Some(profiles) = stored.profiles {
            let profiles = profiles
                .into_iter()
                .filter_map(|profile| {
                    let profile: String = profile
                        .chars()
                        .filter(|character| !character.is_control())
                        .take(32)
                        .collect();
                    let profile = profile.trim().to_owned();
                    (!profile.is_empty()).then_some(profile)
                })
                .fold(Vec::new(), |mut profiles, profile| {
                    if !profiles.contains(&profile) {
                        profiles.push(profile);
                    }
                    profiles
                });
            if !profiles.is_empty() {
                preferences.profiles = profiles;
            }
        }
        if !preferences
            .profiles
            .iter()
            .any(|profile| profile == "Pessoal")
        {
            preferences.profiles.insert(0, "Pessoal".to_owned());
        }
        if let Some(active_profile) = stored.active_profile {
            let active_profile: String = active_profile
                .chars()
                .filter(|character| !character.is_control())
                .take(32)
                .collect();
            let active_profile = active_profile.trim();
            if preferences
                .profiles
                .iter()
                .any(|profile| profile == active_profile)
            {
                preferences.active_profile = active_profile.to_owned();
            }
        }
    } else if let Ok(search_engine) = std::fs::read_to_string(search_engine_path()) {
        // Compatibility with versions that stored only the search engine in a
        // separate file. The JSON file is now the single source of truth.
        preferences.search_engine = SearchEngine::from_identifier(Some(search_engine.trim()));
    }
    preferences
}

fn save_preferences(preferences: &BrowserPreferences) -> Result<(), String> {
    let path = preferences_path();
    let parent = path
        .parent()
        .ok_or_else(|| "diretório das preferências inválido".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("não foi possível criar a pasta das preferências: {error}"))?;
    let stored = StoredPreferences {
        search_engine: Some(preferences.search_engine.id().to_owned()),
        theme: Some(preferences.theme.id().to_owned()),
        show_bookmarks_bar: Some(preferences.show_bookmarks_bar),
        active_profile: Some(preferences.active_profile.clone()),
        profiles: Some(preferences.profiles.clone()),
        https_only: Some(preferences.https_only),
        allow_loopback_http: Some(preferences.allow_loopback_http),
        confirm_http_exceptions: Some(preferences.confirm_http_exceptions),
    };
    let contents = serde_json::to_vec_pretty(&stored)
        .map_err(|error| format!("não foi possível serializar as preferências: {error}"))?;
    let temporary_path = path.with_extension("json.tmp");
    std::fs::write(&temporary_path, contents)
        .map_err(|error| format!("não foi possível gravar as preferências: {error}"))?;
    std::fs::rename(&temporary_path, &path)
        .map_err(|error| format!("não foi possível finalizar as preferências: {error}"))?;

    Ok(())
}

impl Default for BrowserPreferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            search_engine: SearchEngine::DuckDuckGo,
            show_bookmarks_bar: true,
            active_profile: "Pessoal".to_owned(),
            profiles: vec!["Pessoal".to_owned()],
            https_only: true,
            allow_loopback_http: true,
            confirm_http_exceptions: true,
        }
    }
}

pub(crate) type ProfileContexts = Rc<RefCell<HashMap<String, WebContext>>>;
pub(crate) type FaviconCacheDirs = Rc<RefCell<Vec<PathBuf>>>;
pub(crate) type DownloadHistory = Rc<RefCell<Vec<PathBuf>>>;

fn download_history_path() -> PathBuf {
    user_data_root()
        .join("aegis-browser")
        .join("downloads-history.json")
}

pub(crate) fn load_download_history() -> DownloadHistory {
    let path = download_history_path();
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Rc::new(RefCell::new(Vec::new()));
    };
    let paths = serde_json::from_str::<Vec<String>>(&contents)
        .unwrap_or_default()
        .into_iter()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .collect();
    Rc::new(RefCell::new(paths))
}

fn save_download_history(history: &DownloadHistory) -> Result<(), String> {
    let path = download_history_path();
    let parent = path
        .parent()
        .ok_or_else(|| "diretório do histórico inválido".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("não foi possível criar a pasta do histórico: {error}"))?;
    let paths: Vec<String> = history
        .borrow()
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let contents = serde_json::to_vec_pretty(&paths)
        .map_err(|error| format!("não foi possível serializar o histórico: {error}"))?;
    let temporary_path = path.with_extension("json.tmp");
    std::fs::write(&temporary_path, contents)
        .map_err(|error| format!("não foi possível gravar o histórico: {error}"))?;
    std::fs::rename(&temporary_path, &path)
        .map_err(|error| format!("não foi possível finalizar o histórico: {error}"))
}

static FAVICON_CACHE_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn setup_favicon_cache_cleanup(application: &Application) -> FaviconCacheDirs {
    let directories = Rc::new(RefCell::new(Vec::new()));
    let directories_for_shutdown = Rc::clone(&directories);
    application.connect_shutdown(move |_| {
        for directory in directories_for_shutdown.borrow_mut().drain(..) {
            let _ = std::fs::remove_dir_all(directory);
        }
    });
    directories
}

pub(crate) fn new_profile_context(
    profile_id: &str,
    favicon_cache_dirs: &FaviconCacheDirs,
) -> WebContext {
    let profile_name = profile_directory_name(profile_id);
    let data_root = user_data_root()
        .join("aegis-browser")
        .join("profiles")
        .join(&profile_name);
    let cache_root = user_cache_root()
        .join("aegis-browser")
        .join("profiles")
        .join(&profile_name);
    let _ = std::fs::create_dir_all(&data_root);
    let _ = std::fs::create_dir_all(&cache_root);

    let manager = WebsiteDataManager::builder()
        .base_data_directory(data_root.to_string_lossy())
        .base_cache_directory(cache_root.to_string_lossy())
        .build();
    if let Some(cookie_manager) = manager.cookie_manager() {
        let cookie_path = data_root.join("cookies.sqlite");
        if let Some(path) = cookie_path.to_str() {
            cookie_manager.set_persistent_storage(path, CookiePersistentStorage::Sqlite);
        }
        cookie_manager.set_accept_policy(CookieAcceptPolicy::Always);
    }
    let context = WebContext::with_website_data_manager(&manager);
    context.set_cache_model(CacheModel::WebBrowser);
    context.set_automation_allowed(false);

    // WebKitGTK 4.1 only populates WebView::favicon after its favicon
    // database is enabled. Keep the database in a private, process-specific
    // temporary directory and remove it when the application shuts down.
    let sequence = FAVICON_CACHE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "aegis-browser-favicons-{}-{}",
        std::process::id(),
        sequence
    ));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    if builder.create(&directory).is_ok() {
        if let Some(path) = directory.to_str() {
            context.set_favicon_database_directory(Some(path));
            favicon_cache_dirs.borrow_mut().push(directory);
        }
    }

    context
}

fn profile_directory_name(profile_id: &str) -> String {
    let sanitized: String = profile_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "profile".to_owned()
    } else {
        sanitized
    }
}

fn user_data_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
}

fn user_cache_root() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(|| PathBuf::from(".cache"))
}

impl NavigationEngine for GtkEngine {
    fn load(
        &mut self,
        _tab_id: TabId,
        url: &WebUrl,
    ) -> Result<(), aegis_browser_shell::EngineError> {
        self.approved_loads
            .borrow_mut()
            .insert(url.as_str().to_owned());
        // WebKit may emit policy/load signals synchronously from `load_uri`.
        // Defer the call one GTK turn so those callbacks cannot re-enter the
        // BrowserShell while navigate_with_engine still holds its borrow.
        let webview = self.webview.clone();
        let uri = url.as_str().to_owned();
        glib::idle_add_local_once(move || webview.load_uri(&uri));
        Ok(())
    }
}

mod bitwarden;
mod extensions;
mod tabs;

const APPLICATION_ID: &str = "io.github.n8nfelipe.aegis-browser";

fn main() {
    // GTK 3 uses the program name when publishing the Wayland app_id. Keep it
    // identical to the GtkApplication ID and the desktop-entry filename so
    // the compositor can associate the running window with its launcher.
    glib::set_prgname(Some(APPLICATION_ID));
    let application = Application::new(Some(APPLICATION_ID), Default::default());
    application.connect_activate(tabs::build_ui);
    application.run();
}

#[allow(dead_code)]
fn _legacy_build_ui(application: &Application) {
    let policy_state = Rc::new(RefCell::new(BrowserPolicy::default()));
    let shell = Rc::new(RefCell::new(BrowserShell::new(*policy_state.borrow())));
    let preferences = Rc::new(RefCell::new(BrowserPreferences::default()));
    let tab_id = shell.borrow_mut().new_tab();

    // Ephemeral context means this first backend does not persist cookies,
    // cache or other WebKit website data between runs.
    let favicon_cache_dirs = setup_favicon_cache_cleanup(application);
    let context = new_profile_context("Pessoal", &favicon_cache_dirs);
    let download_history: DownloadHistory = load_download_history();
    let profile_contexts: ProfileContexts = Rc::new(RefCell::new(HashMap::from([(
        "Pessoal".to_owned(),
        context.clone(),
    )])));
    let webview = WebView::with_context(&context);

    let settings = Settings::new();
    settings.set_enable_developer_extras(false);
    settings.set_enable_javascript(true);
    settings.set_user_agent(Some(COMPATIBLE_USER_AGENT));
    // Allow sites such as Codex to copy text through the JavaScript Clipboard API.
    settings.set_javascript_can_access_clipboard(true);
    settings.set_enable_webgl(true);
    settings.set_enable_smooth_scrolling(true);
    settings.set_enable_media(true);
    settings.set_enable_media_capabilities(true);
    settings.set_enable_media_stream(true);
    settings.set_enable_mediasource(true);
    settings.set_enable_webaudio(true);
    settings.set_media_playback_allows_inline(true);
    settings.set_media_playback_requires_user_gesture(true);
    settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::OnDemand);
    webview.set_settings(&settings);

    let approved_loads = Rc::new(RefCell::new(HashSet::new()));
    let engine = Rc::new(RefCell::new(GtkEngine {
        webview: webview.clone(),
        approved_loads: Rc::clone(&approved_loads),
    }));
    let window = ApplicationWindow::new(application);
    window.set_title("Aegis Browser — WebKitGTK");
    window.set_default_size(1200, 800);

    let root = GtkBox::new(Orientation::Vertical, 0);
    let toolbar = GtkBox::new(Orientation::Horizontal, 6);
    toolbar.set_margin_top(8);
    toolbar.set_margin_bottom(8);
    toolbar.set_margin_start(8);
    toolbar.set_margin_end(8);

    let address = Entry::new();
    address.set_placeholder_text(Some("Digite uma URL HTTPS"));
    address.set_hexpand(true);
    let go = Button::with_label("Ir");
    let settings = Button::with_label("Configurações");
    let security = Label::new(Some("Nova aba"));
    let status = Label::new(Some("Pronto"));
    let progress = ProgressBar::new();
    progress.set_show_text(false);
    progress.set_fraction(0.0);

    toolbar.pack_start(&security, false, false, 0);
    toolbar.pack_start(&address, true, true, 0);
    toolbar.pack_start(&go, false, false, 0);
    toolbar.pack_start(&settings, false, false, 0);
    toolbar.pack_start(&status, false, false, 0);
    root.pack_start(&toolbar, false, false, 0);
    root.pack_start(&progress, false, false, 0);
    root.pack_start(&webview, true, true, 0);
    window.add(&root);

    // WebKit can request a top-level navigation without going through the
    // address bar (redirects, history, links and new-window actions). Keep
    // those requests behind the same HTTPS policy as user-initiated loads.
    let policy_for_navigation = Rc::clone(&policy_state);
    let approved_for_navigation = Rc::clone(&approved_loads);
    webview.connect_decide_policy(move |_view, decision, decision_type| {
        match decision_type {
            PolicyDecisionType::NewWindowAction => {
                // Aegis has no popup/tab creation path yet. Do not let a page
                // create an unmanaged WebView or window.
                decision.ignore();
                true
            }
            PolicyDecisionType::NavigationAction => {
                let Some(navigation) = decision.downcast_ref::<NavigationPolicyDecision>() else {
                    decision.ignore();
                    return true;
                };
                let Some(request) = navigation
                    .navigation_action()
                    .and_then(|action| action.request())
                else {
                    decision.ignore();
                    return true;
                };
                let Some(uri) = request.uri() else {
                    decision.ignore();
                    return true;
                };
                let uri = uri.to_string();

                if approved_for_navigation.borrow_mut().remove(&uri) {
                    decision.use_();
                    return true;
                }

                let allowed = WebUrl::parse(&uri)
                    .ok()
                    .map(|url| {
                        policy_for_navigation
                            .borrow()
                            .evaluate_navigation(NavigationRequest {
                                scheme: url.origin().scheme(),
                                is_loopback: url.is_loopback(),
                                is_top_level: true,
                                has_user_gesture: false,
                            })
                            == NavigationDecision::Allow
                    })
                    .unwrap_or(false);

                if allowed {
                    decision.use_();
                } else {
                    decision.ignore();
                }
                true
            }
            // Response decisions also cover subresources. Returning false
            // preserves WebKit's normal resource handling; top-level policy
            // is enforced by NavigationAction above.
            PolicyDecisionType::Response | PolicyDecisionType::__Unknown(_) => false,
            _ => false,
        }
    });

    configure_download_policy(
        &context,
        &window,
        status.clone(),
        None,
        Rc::clone(&download_history),
    );

    let window_for_settings = window.clone();
    let shell_for_settings = Rc::clone(&shell);
    let policy_for_settings = Rc::clone(&policy_state);
    let preferences_for_settings = Rc::clone(&preferences);
    let profile_contexts_for_settings = Rc::clone(&profile_contexts);
    let favicon_cache_dirs_for_settings = Rc::clone(&favicon_cache_dirs);
    let status_for_settings = status.clone();
    settings.connect_clicked(move |_| {
        show_settings_dialog(
            &window_for_settings,
            Rc::clone(&shell_for_settings),
            Rc::clone(&policy_for_settings),
            Rc::clone(&preferences_for_settings),
            Rc::clone(&profile_contexts_for_settings),
            Rc::clone(&favicon_cache_dirs_for_settings),
            status_for_settings.clone(),
            None,
            None,
            None,
            None,
            Rc::clone(&download_history),
        );
    });

    let navigate: Rc<dyn Fn()> = Rc::new({
        let shell = Rc::clone(&shell);
        let engine = Rc::clone(&engine);
        let address = address.clone();
        let security = security.clone();
        let status = status.clone();
        let window = window.clone();
        move || {
            let input = address.text().trim().to_owned();
            if input.is_empty() {
                status.set_text("Digite um endereço");
                return;
            }

            let result = shell.borrow_mut().navigate_with_engine(
                &mut *engine.borrow_mut(),
                tab_id,
                &input,
                true,
            );
            match result {
                Ok(NavigationResult::Navigated { .. }) => {
                    security.set_text("HTTPS/HTTP verificado");
                    status.set_text("Carregando");
                }
                Ok(NavigationResult::BlockedInsecure { .. }) => {
                    security.set_text("HTTP bloqueado");
                    status.set_text("Navegação insegura bloqueada");
                }
                Ok(NavigationResult::ConfirmationRequired { tab_id, .. }) => {
                    show_http_confirmation(
                        &window,
                        Rc::clone(&shell),
                        Rc::clone(&engine),
                        tab_id,
                        status.clone(),
                    );
                }
                Err(error) => status.set_text(&shell_error_text(error)),
            }
        }
    });

    let navigate_for_button = Rc::clone(&navigate);
    go.connect_clicked(move |_| navigate_for_button());
    address.connect_activate(move |_| navigate());

    let shell_for_events = Rc::clone(&shell);
    let status_for_events = status.clone();
    let progress_for_events = progress.clone();
    webview.connect_load_changed(move |_view, event| {
        let engine_event = match event {
            LoadEvent::Started => Some(EngineEvent::LoadStarted { tab_id }),
            LoadEvent::Finished => Some(EngineEvent::LoadFinished { tab_id }),
            _ => None,
        };
        if let Some(engine_event) = engine_event {
            if shell_for_events
                .borrow_mut()
                .handle_engine_event(engine_event)
                .is_err()
            {
                status_for_events.set_text("Evento inválido da engine");
            } else if event == LoadEvent::Finished {
                progress_for_events.set_fraction(1.0);
                status_for_events.set_text("Carregado");
            } else {
                progress_for_events.set_fraction(0.0);
                status_for_events.set_text("Carregando");
            }
        }
    });

    let shell_for_title = Rc::clone(&shell);
    let status_for_title = status.clone();
    webview.connect_title_notify(move |view| {
        if let Some(title) = view.title() {
            if shell_for_title
                .borrow_mut()
                .handle_title_changed(TitleChanged {
                    tab_id,
                    title: title.to_string(),
                })
                .is_err()
            {
                status_for_title.set_text("Título inválido");
            }
        }
    });

    let progress_for_notify = progress.clone();
    webview.connect_estimated_load_progress_notify(move |view| {
        progress_for_notify.set_fraction(view.estimated_load_progress());
    });

    let shell_for_failure = Rc::clone(&shell);
    let status_for_failure = status.clone();
    webview.connect_load_failed(move |_view, _event, _failing_uri, _error| {
        let result = shell_for_failure
            .borrow_mut()
            .handle_engine_event(EngineEvent::LoadFailed {
                tab_id,
                error: LoadError::Unknown,
            });
        if result.is_err() {
            status_for_failure.set_text("Falha de carregamento inválida");
        } else {
            status_for_failure.set_text("Falha ao carregar");
        }
        false
    });

    window.show_all();
}

fn show_settings_dialog(
    window: &ApplicationWindow,
    shell: Rc<RefCell<BrowserShell>>,
    policy_state: Rc<RefCell<BrowserPolicy>>,
    preferences: Rc<RefCell<BrowserPreferences>>,
    profile_contexts: ProfileContexts,
    favicon_cache_dirs: FaviconCacheDirs,
    status: Label,
    extension_toolbar: Option<GtkBox>,
    bookmarks_bar: Option<GtkBox>,
    devtools_action: Option<Rc<dyn Fn()>>,
    downloads_button: Option<Button>,
    download_history: DownloadHistory,
) {
    let current = *policy_state.borrow();
    let current_preferences = preferences.borrow().clone();
    let dialog = Dialog::with_buttons(
        Some("Configurações"),
        Some(window),
        DialogFlags::MODAL | DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Cancelar", ResponseType::Cancel),
            ("Aplicar", ResponseType::Accept),
        ],
    );
    dialog.set_default_size(720, 560);

    let content = dialog.content_area();
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let notebook = Notebook::new();
    notebook.set_tab_pos(PositionType::Left);
    notebook.set_scrollable(true);

    let privacy_page = settings_page(
        "Privacidade e segurança",
        "Controles aplicados ao shell de navegação desta execução.",
    );
    let privacy_grid = Grid::new();
    privacy_grid.set_column_spacing(18);
    privacy_grid.set_row_spacing(14);

    let https_only = CheckButton::with_label("Exigir HTTPS");
    https_only.set_active(current.https_only);
    privacy_grid.attach(&https_only, 0, 0, 2, 1);

    let http_exceptions = CheckButton::with_label("Pedir confirmação para exceções HTTP");
    http_exceptions.set_active(current.confirm_http_exceptions);
    http_exceptions.set_sensitive(!current.https_only);
    privacy_grid.attach(&http_exceptions, 0, 1, 2, 1);

    let http_exceptions_for_toggle = http_exceptions.clone();
    https_only.connect_toggled(move |check| {
        http_exceptions_for_toggle.set_sensitive(!check.is_active());
    });

    let loopback_http = CheckButton::with_label("Permitir HTTP em localhost");
    loopback_http.set_active(current.allow_loopback_http);
    privacy_grid.attach(&loopback_http, 0, 2, 2, 1);

    let https_help = Label::new(Some(
        "HTTP público continua bloqueado quando esta opção está ativa.",
    ));
    https_help.set_xalign(0.0);
    https_help.set_line_wrap(true);
    privacy_grid.attach(&https_help, 0, 3, 2, 1);

    let ephemeral = Label::new(Some(
        "Contexto efêmero: cookies e cache não são preservados entre execuções.",
    ));
    ephemeral.set_xalign(0.0);
    ephemeral.set_line_wrap(true);
    privacy_grid.attach(&ephemeral, 0, 5, 2, 1);

    let downloads = Label::new(Some(
        "Downloads: bloqueados até você autorizar e escolher o destino.",
    ));
    downloads.set_xalign(0.0);
    privacy_grid.attach(&downloads, 0, 6, 2, 1);
    privacy_page.pack_start(&privacy_grid, false, false, 0);
    notebook.append_page(&privacy_page, Some(&Label::new(Some("Privacidade"))));

    let appearance_page = settings_page(
        "Aparência",
        "Escolha como a interface do Aegis deve se apresentar.",
    );
    let appearance_grid = Grid::new();
    appearance_grid.set_column_spacing(18);
    appearance_grid.set_row_spacing(14);
    let theme_label = Label::new(Some("Tema da interface"));
    theme_label.set_xalign(0.0);
    let theme = ComboBoxText::new();
    for option in [
        ThemePreference::System,
        ThemePreference::Light,
        ThemePreference::Dark,
    ] {
        theme.append(Some(option.id()), option.label());
    }
    theme.set_width_request(220);
    theme.set_active_id(Some(current_preferences.theme.id()));
    appearance_grid.attach(&theme_label, 0, 0, 1, 1);
    appearance_grid.attach(&theme, 1, 0, 1, 1);
    let theme_help = Label::new(Some(
        "A preferência vale para a interface GTK e não altera o tema dos sites.",
    ));
    theme_help.set_xalign(0.0);
    theme_help.set_line_wrap(true);
    appearance_grid.attach(&theme_help, 0, 1, 2, 1);
    let bookmarks_bar_toggle = CheckButton::with_label("Mostrar barra de favoritos");
    bookmarks_bar_toggle.set_active(current_preferences.show_bookmarks_bar);
    appearance_grid.attach(&bookmarks_bar_toggle, 0, 2, 2, 1);
    let bookmarks_help = Label::new(Some(
        "A barra aparece abaixo da navegação. Use a estrela ao lado do endereço para salvar a página atual.",
    ));
    bookmarks_help.set_xalign(0.0);
    bookmarks_help.set_line_wrap(true);
    appearance_grid.attach(&bookmarks_help, 0, 3, 2, 1);
    appearance_page.pack_start(&appearance_grid, false, false, 0);
    notebook.append_page(&appearance_page, Some(&Label::new(Some("Aparência"))));

    let search_page = settings_page(
        "Busca e navegação",
        "Texto que não parece um endereço será enviado ao buscador escolhido.",
    );
    let search_grid = Grid::new();
    search_grid.set_column_spacing(18);
    search_grid.set_row_spacing(14);
    let search_label = Label::new(Some("Buscador padrão"));
    search_label.set_xalign(0.0);
    let search_engine = ComboBoxText::new();
    for option in [
        SearchEngine::DuckDuckGo,
        SearchEngine::Brave,
        SearchEngine::Startpage,
        SearchEngine::Qwant,
        SearchEngine::Bing,
        SearchEngine::Google,
    ] {
        search_engine.append(Some(option.id()), option.label());
    }
    search_engine.set_width_request(220);
    search_engine.set_active_id(Some(current_preferences.search_engine.id()));
    search_grid.attach(&search_label, 0, 0, 1, 1);
    search_grid.attach(&search_engine, 1, 0, 1, 1);
    let search_help = Label::new(Some(
        "O Aegis não adiciona identificadores próprios à consulta. O buscador ainda poderá aplicar sua política de privacidade.",
    ));
    search_help.set_xalign(0.0);
    search_help.set_line_wrap(true);
    search_grid.attach(&search_help, 0, 1, 2, 1);
    search_page.pack_start(&search_grid, false, false, 0);
    notebook.append_page(&search_page, Some(&Label::new(Some("Buscadores"))));

    let profiles_page = settings_page(
        "Perfis",
        "Cada perfil novo recebe um contexto efêmero separado para novas abas.",
    );
    let profiles_grid = Grid::new();
    profiles_grid.set_column_spacing(12);
    profiles_grid.set_row_spacing(14);
    let active_profile_label = Label::new(Some("Perfil para novas abas"));
    active_profile_label.set_xalign(0.0);
    let active_profile = ComboBoxText::new();
    for profile in &current_preferences.profiles {
        active_profile.append(Some(profile), profile);
    }
    active_profile.set_active_id(Some(&current_preferences.active_profile));
    profiles_grid.attach(&active_profile_label, 0, 0, 1, 1);
    profiles_grid.attach(&active_profile, 1, 0, 1, 1);

    let new_profile = Entry::new();
    new_profile.set_placeholder_text(Some("Nome do novo perfil"));
    let add_profile = Button::with_label("Criar perfil");
    profiles_grid.attach(&new_profile, 0, 1, 1, 1);
    profiles_grid.attach(&add_profile, 1, 1, 1, 1);
    let profile_names = Rc::new(RefCell::new(current_preferences.profiles.clone()));
    let profile_names_for_add = Rc::clone(&profile_names);
    let active_profile_for_add = active_profile.clone();
    let new_profile_for_add = new_profile.clone();
    add_profile.connect_clicked(move |_| {
        let name: String = new_profile_for_add
            .text()
            .chars()
            .filter(|character| !character.is_control())
            .take(32)
            .collect();
        let name = name.trim().to_owned();
        if name.is_empty()
            || profile_names_for_add
                .borrow()
                .iter()
                .any(|item| item == &name)
        {
            return;
        }
        profile_names_for_add.borrow_mut().push(name.clone());
        active_profile_for_add.append(Some(&name), &name);
        active_profile_for_add.set_active_id(Some(&name));
        new_profile_for_add.set_text("");
    });

    let profile_help = Label::new(Some(
        "Abas já abertas permanecem no perfil original. Crie um perfil separado para trabalho, testes ou uso compartilhado.",
    ));
    profile_help.set_xalign(0.0);
    profile_help.set_line_wrap(true);
    profiles_grid.attach(&profile_help, 0, 2, 2, 1);
    profiles_page.pack_start(&profiles_grid, false, false, 0);
    notebook.append_page(&profiles_page, Some(&Label::new(Some("Perfis"))));

    let extensions_page = settings_page(
        "Extensões",
        "Adicione uma extensão descompactada contendo manifest.json.",
    );
    let extensions_box = GtkBox::new(Orientation::Vertical, 12);
    let add_extension = Button::with_label("Selecionar manifest.json…");
    let extension_path = Entry::new();
    extension_path.set_hexpand(true);
    extension_path.set_placeholder_text(Some("Caminho da pasta ou do manifest.json"));
    let add_extension_path = Button::with_label("Adicionar caminho");
    let extension_path_row = GtkBox::new(Orientation::Horizontal, 8);
    extension_path_row.pack_start(&extension_path, true, true, 0);
    extension_path_row.pack_start(&add_extension_path, false, false, 0);
    let extensions_help = Label::new(Some(
        "São aceitos content scripts JavaScript/CSS para páginas HTTP e HTTPS. Extensões completas da Chrome Web Store ainda não são compatíveis.",
    ));
    extensions_help.set_xalign(0.0);
    extensions_help.set_line_wrap(true);
    let extension_status = Label::new(None);
    extension_status.set_xalign(0.0);
    extension_status.set_line_wrap(true);
    let extensions_list = GtkBox::new(Orientation::Vertical, 8);
    refresh_extensions_list(
        &extensions_list,
        &extension_status,
        extension_toolbar.as_ref(),
    );
    extensions_box.pack_start(&add_extension, false, false, 0);
    extensions_box.pack_start(&extension_path_row, false, false, 0);
    extensions_box.pack_start(&extensions_help, false, false, 0);
    extensions_box.pack_start(&extension_status, false, false, 0);
    extensions_box.pack_start(&extensions_list, false, false, 0);
    extensions_page.pack_start(&extensions_box, false, false, 0);
    notebook.append_page(&extensions_page, Some(&Label::new(Some("Extensões"))));

    if let Some(show_devtools) = devtools_action {
        let development_page = settings_page(
            "Desenvolvimento",
            "Inspecione a página ativa com as ferramentas de desenvolvimento do WebKit.",
        );
        let open_devtools = Button::with_label("Abrir DevTools");
        open_devtools.set_tooltip_text(Some("Abrir o inspector da aba ativa"));
        let dialog_for_devtools = dialog.clone();
        open_devtools.connect_clicked(move |_| {
            dialog_for_devtools.close();
            show_devtools();
        });
        development_page.pack_start(&open_devtools, false, false, 0);
        notebook.append_page(
            &development_page,
            Some(&Label::new(Some("Desenvolvimento"))),
        );
    }

    let window_for_extension_chooser = window.clone();
    let extensions_list_for_add = extensions_list.clone();
    let extension_status_for_add = extension_status.clone();
    let extension_path_for_chooser = extension_path.clone();
    let extension_toolbar_for_chooser = extension_toolbar.clone();
    let extension_chooser_holder: Rc<RefCell<Option<FileChooserDialog>>> =
        Rc::new(RefCell::new(None));
    add_extension.connect_clicked(move |_| {
        let manifest_filter = FileFilter::new();
        manifest_filter.set_name(Some("manifest.json"));
        manifest_filter.add_pattern("manifest.json");
        let chooser = FileChooserDialog::with_buttons(
            Some("Selecionar manifest.json da extensão"),
            Some(&window_for_extension_chooser),
            FileChooserAction::Open,
            &[
                ("Cancelar", ResponseType::Cancel),
                ("Adicionar", ResponseType::Accept),
            ],
        );
        chooser.set_modal(true);
        chooser.set_default_size(800, 500);
        chooser.set_select_multiple(true);
        chooser.set_filter(&manifest_filter);
        *extension_chooser_holder.borrow_mut() = Some(chooser.clone());
        chooser.connect_response({
            let extensions_list = extensions_list_for_add.clone();
            let extension_status = extension_status_for_add.clone();
            let extension_path = extension_path_for_chooser.clone();
            let extension_toolbar = extension_toolbar_for_chooser.clone();
            let extension_chooser_holder = extension_chooser_holder.clone();
            move |chooser, response| {
                if response == ResponseType::Accept {
                    let selected_paths = chooser.filenames();
                    if selected_paths.is_empty() {
                        extension_status.set_text("Nenhum arquivo selecionado.");
                    } else {
                        let mut added = 0;
                        let mut already_installed = 0;
                        let mut errors = Vec::new();
                        let mut first_manifest = None;

                        for path in selected_paths {
                            if path.file_name().and_then(|name| name.to_str())
                                != Some("manifest.json")
                            {
                                errors.push(format!("{} não é manifest.json", path.display()));
                                continue;
                            }
                            if first_manifest.is_none() {
                                first_manifest = Some(path.clone());
                            }
                            match import_extension_path(path) {
                                Ok(_) => added += 1,
                                Err(error) if error.contains("já está instalada") => {
                                    already_installed += 1;
                                }
                                Err(error) => errors.push(error),
                            }
                        }

                        if let Some(path) = first_manifest {
                            extension_path.set_text(&path.to_string_lossy());
                        }
                        refresh_extensions_list(
                            &extensions_list,
                            &extension_status,
                            extension_toolbar.as_ref(),
                        );
                        if let Some(toolbar) = extension_toolbar.as_ref() {
                            refresh_extensions_toolbar(toolbar);
                        }

                        let mut status = Vec::new();
                        if added > 0 {
                            status.push(format!("{added} extensão(ões) adicionada(s)"));
                        }
                        if already_installed > 0 {
                            status
                                .push(format!("{already_installed} extensão(ões) já instalada(s)"));
                        }
                        if !errors.is_empty() {
                            status.push(format!("Erros: {}", errors.join("; ")));
                        }
                        extension_status.set_text(&status.join(". "));
                    }
                }
                chooser.close();
                extension_chooser_holder.borrow_mut().take();
            }
        });
        chooser.show_all();
    });

    let extensions_list_for_path = extensions_list.clone();
    let extension_status_for_path = extension_status.clone();
    let extension_path_for_add = extension_path.clone();
    let extension_toolbar_for_path = extension_toolbar.clone();
    add_extension_path.connect_clicked(move |_| {
        let input = extension_path_for_add.text().trim().to_owned();
        match import_extension_path(PathBuf::from(input)) {
            Ok(extension) => {
                extension_status_for_path.set_text(&format!(
                    "Extensão adicionada: {} ({})",
                    extension.name, extension.version
                ));
                extension_path_for_add.set_text("");
                refresh_extensions_list(
                    &extensions_list_for_path,
                    &extension_status_for_path,
                    extension_toolbar_for_path.as_ref(),
                );
                if let Some(toolbar) = extension_toolbar_for_path.as_ref() {
                    refresh_extensions_toolbar(toolbar);
                }
            }
            Err(error) if error.contains("já está instalada") => {
                extension_status_for_path.set_text("Essa extensão já está instalada.");
                extension_path_for_add.set_text("");
                refresh_extensions_list(
                    &extensions_list_for_path,
                    &extension_status_for_path,
                    extension_toolbar_for_path.as_ref(),
                );
                if let Some(toolbar) = extension_toolbar_for_path.as_ref() {
                    refresh_extensions_toolbar(toolbar);
                }
            }
            Err(error) => extension_status_for_path
                .set_text(&format!("Não foi possível adicionar a extensão: {error}")),
        }
    });

    content.add(&notebook);

    let shell_for_response = shell;
    let policy_for_response = policy_state;
    let preferences_for_response = preferences;
    let profile_contexts_for_response = profile_contexts;
    let favicon_cache_dirs_for_response = favicon_cache_dirs;
    let bookmarks_bar_for_response = bookmarks_bar;
    let window_for_downloads = window.clone();
    let downloads_button_for_response = downloads_button;
    let download_history_for_response = download_history;
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            let mut policy = *policy_for_response.borrow();
            policy.https_only = https_only.is_active();
            policy.confirm_http_exceptions = http_exceptions.is_active();
            policy.allow_loopback_http = loopback_http.is_active();
            *policy_for_response.borrow_mut() = policy;
            shell_for_response.borrow_mut().set_policy(policy);

            let selected_profile = active_profile
                .active_text()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "Pessoal".to_owned());
            let profiles = profile_names.borrow().clone();
            for profile in &profiles {
                if !profile_contexts_for_response.borrow().contains_key(profile) {
                    let context = new_profile_context(profile, &favicon_cache_dirs_for_response);
                    configure_download_policy(
                        &context,
                        &window_for_downloads,
                        status.clone(),
                        downloads_button_for_response.clone(),
                        Rc::clone(&download_history_for_response),
                    );
                    profile_contexts_for_response
                        .borrow_mut()
                        .insert(profile.clone(), context);
                }
            }
            let selected_theme = ThemePreference::from_id(theme.active_id());
            let selected_search_engine = SearchEngine::from_id(search_engine.active_id());
            let show_bookmarks_bar = bookmarks_bar_toggle.is_active();
            *preferences_for_response.borrow_mut() = BrowserPreferences {
                theme: selected_theme,
                search_engine: selected_search_engine,
                show_bookmarks_bar,
                active_profile: selected_profile.clone(),
                profiles,
                https_only: policy.https_only,
                allow_loopback_http: policy.allow_loopback_http,
                confirm_http_exceptions: policy.confirm_http_exceptions,
            };
            let preferences_snapshot = preferences_for_response.borrow().clone();
            let preferences_save_result = save_preferences(&preferences_snapshot);
            if let Some(bookmarks_bar) = bookmarks_bar_for_response.as_ref() {
                bookmarks_bar.set_visible(show_bookmarks_bar);
            }
            apply_theme(selected_theme);
            match preferences_save_result {
                Ok(()) => status.set_text(&format!(
                    "Configurações aplicadas e salvas — novas abas: {selected_profile}"
                )),
                Err(error) => status.set_text(&format!(
                    "Configurações aplicadas nesta sessão; não foi possível salvar: {error}"
                )),
            }
        }
        dialog.close();
    });

    dialog.show_all();
}

fn settings_page(title: &str, description: &str) -> GtkBox {
    let page = GtkBox::new(Orientation::Vertical, 18);
    page.set_margin_top(18);
    page.set_margin_bottom(18);
    page.set_margin_start(18);
    page.set_margin_end(18);

    let heading = Label::new(Some(title));
    heading.set_xalign(0.0);
    let description_label = Label::new(Some(description));
    description_label.set_xalign(0.0);
    description_label.set_line_wrap(true);
    page.pack_start(&heading, false, false, 0);
    page.pack_start(&description_label, false, false, 0);
    page
}

pub(crate) fn refresh_extensions_toolbar(container: &GtkBox) {
    for child in container.children() {
        container.remove(&child);
    }

    let parent = container
        .toplevel()
        .and_then(|widget| widget.downcast::<ApplicationWindow>().ok());
    for extension in extensions::installed_extensions() {
        let button = Button::new();
        button.set_focus_on_click(false);
        button.set_tooltip_text(Some(&format!(
            "{} — versão {}",
            extension.name, extension.version
        )));
        let image = extension
            .icon_path
            .as_ref()
            .and_then(|path| gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 22, 22, true).ok())
            .map(|pixbuf| Image::from_pixbuf(Some(&pixbuf)))
            .unwrap_or_else(|| {
                Image::from_icon_name(Some("application-x-addon"), IconSize::Button)
            });
        image.set_pixel_size(22);
        button.set_image(Some(&image));
        button.set_always_show_image(true);
        let extension_for_button = extension.clone();
        if let Some(parent_for_button) = parent.clone() {
            button.connect_clicked(move |_| {
                show_extension_popup(&parent_for_button, &extension_for_button);
            });
        }
        container.pack_start(&button, false, false, 0);
    }
    container.show_all();
}

fn show_extension_popup(parent: &ApplicationWindow, extension: &extensions::InstalledExtension) {
    let Some(popup_path) = extension.popup_path.as_ref() else {
        let dialog = MessageDialog::new(
            Some(parent),
            DialogFlags::MODAL,
            MessageType::Info,
            ButtonsType::Close,
            &format!(
                "A extensão {} não possui um popup configurado.",
                extension.name
            ),
        );
        dialog.connect_response(|dialog, _| dialog.close());
        dialog.show_all();
        return;
    };

    let Ok(uri) = glib::filename_to_uri(popup_path, None) else {
        return;
    };
    let popup = Window::new(WindowType::Toplevel);
    popup.set_title(&format!("Extensão — {}", extension.name));
    popup.set_transient_for(Some(parent));
    popup.set_modal(true);
    popup.set_default_size(360, 480);
    let webview = WebView::new();
    webview.load_uri(uri.as_str());
    popup.add(&webview);
    popup.show_all();
}

fn refresh_extensions_list(container: &GtkBox, status: &Label, extension_toolbar: Option<&GtkBox>) {
    for child in container.children() {
        container.remove(&child);
    }

    let installed = extensions::installed_extensions();
    if installed.is_empty() {
        let empty = Label::new(Some("Nenhuma extensão instalada."));
        empty.set_xalign(0.0);
        container.pack_start(&empty, false, false, 0);
    } else {
        for extension in installed {
            let row = GtkBox::new(Orientation::Horizontal, 8);
            let label = Label::new(Some(&format!(
                "{} — versão {} ({})",
                extension.name, extension.version, extension.id
            )));
            label.set_xalign(0.0);
            label.set_line_wrap(true);
            label.set_hexpand(true);
            let remove = Button::with_label("Remover");
            remove.set_tooltip_text(Some("Desinstalar esta extensão"));
            let container_for_remove = container.clone();
            let status_for_remove = status.clone();
            let toolbar_for_remove = extension_toolbar.cloned();
            let extension_id = extension.id.clone();
            let extension_name = extension.name.clone();
            remove.connect_clicked(
                move |_| match extensions::uninstall_extension(&extension_id) {
                    Ok(()) => {
                        status_for_remove.set_text(&format!("Extensão removida: {extension_name}"));
                        refresh_extensions_list(
                            &container_for_remove,
                            &status_for_remove,
                            toolbar_for_remove.as_ref(),
                        );
                        if let Some(toolbar) = toolbar_for_remove.as_ref() {
                            refresh_extensions_toolbar(toolbar);
                        }
                    }
                    Err(error) => status_for_remove
                        .set_text(&format!("Não foi possível remover a extensão: {error}")),
                },
            );
            row.pack_start(&label, true, true, 0);
            row.pack_start(&remove, false, false, 0);
            container.pack_start(&row, false, false, 0);
        }
    }
    container.show_all();
}

fn import_extension_path(mut path: PathBuf) -> Result<extensions::InstalledExtension, String> {
    if let Some(path_text) = path.to_str() {
        if let Some(relative) = path_text.strip_prefix("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                path = PathBuf::from(home).join(relative);
            }
        }
    }

    if path.is_file() {
        if path.file_name().and_then(|name| name.to_str()) != Some("manifest.json") {
            return Err("selecione o arquivo manifest.json".to_owned());
        }
        path = path
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| "a pasta da extensão não foi encontrada".to_owned())?;
    }
    if !path.is_dir() {
        return Err("o caminho não é uma pasta de extensão válida".to_owned());
    }
    extensions::import_unpacked_extension(&path)
}

fn apply_theme(theme: ThemePreference) {
    if let Some(settings) = gtk::Settings::default() {
        match theme {
            ThemePreference::System => {
                // Restore GTK's configured theme instead of leaving behind a
                // forced Adwaita variant from a previous selection.
                settings.set_gtk_theme_name(None);
                settings.set_gtk_application_prefer_dark_theme(false);
            }
            ThemePreference::Light => {
                // Turning the dark preference off is not enough when the
                // desktop theme itself is already a dark variant.
                settings.set_gtk_theme_name(Some("Adwaita"));
                settings.set_gtk_application_prefer_dark_theme(false);
            }
            ThemePreference::Dark => {
                settings.set_gtk_theme_name(Some("Adwaita:dark"));
                settings.set_gtk_application_prefer_dark_theme(true);
            }
        }
    }
}

fn show_download_destination_chooser(
    window: &ApplicationWindow,
    source_url: &str,
    suggested_filename: &str,
    status: &Label,
    downloads_button: Option<Button>,
    download_history: DownloadHistory,
) {
    let dialog = Dialog::with_buttons(
        Some("Salvar download"),
        Some(window),
        DialogFlags::MODAL | DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Cancelar", ResponseType::Cancel),
            ("Salvar", ResponseType::Accept),
        ],
    );
    dialog.set_default_size(640, 150);

    let content = dialog.content_area();
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    let help = Label::new(Some(
        "Informe o caminho completo do arquivo ou uma pasta existente.",
    ));
    help.set_xalign(0.0);
    let default_directory = default_download_directory();
    let _ = std::fs::create_dir_all(&default_directory);
    let destination_entry = Entry::new();
    destination_entry.set_hexpand(true);
    destination_entry.set_text(&default_directory.to_string_lossy());
    content.pack_start(&help, false, false, 0);
    content.pack_start(&destination_entry, false, false, 0);

    let source_url = source_url.to_owned();
    let status_for_response = status.clone();
    let suggested_filename = suggested_filename.to_owned();
    let response_handled = Rc::new(Cell::new(false));
    let response_handled_for_response = Rc::clone(&response_handled);
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            if response_handled_for_response.get() {
                return;
            }
            let raw_destination = destination_entry.text().trim().to_owned();
            if raw_destination.is_empty() {
                status_for_response.set_text("Informe um destino para o download");
                return;
            }
            let destination = expand_user_path(&raw_destination);
            let destination = if destination.is_dir() {
                destination.join(&suggested_filename)
            } else {
                destination
            };
            let Some(parent) = destination.parent() else {
                response_handled_for_response.set(true);
                dialog.close();
                status_for_response.set_text("Destino de download inválido");
                return;
            };
            if let Err(error) = std::fs::create_dir_all(parent) {
                response_handled_for_response.set(true);
                dialog.close();
                status_for_response.set_text(&format!(
                    "Não foi possível criar a pasta do download: {error}"
                ));
                return;
            }
            response_handled_for_response.set(true);
            dialog.close();
            start_external_download(
                &source_url,
                &destination,
                &status_for_response,
                downloads_button.as_ref(),
                &download_history,
            );
        } else if !response_handled_for_response.replace(true) {
            dialog.close();
            status_for_response.set_text("Download cancelado");
        }
    });
    dialog.show_all();
}

fn default_download_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads")
}

fn expand_user_path(path: &str) -> PathBuf {
    if let Some(relative) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(relative);
        }
    }
    PathBuf::from(path)
}

pub(crate) fn suggested_filename_from_url(source_url: &str) -> String {
    source_url
        .split('?')
        .next()
        .and_then(|url| url.rsplit('/').next())
        .filter(|filename| !filename.is_empty())
        .unwrap_or("download")
        .to_owned()
}

pub(crate) fn safe_download_filename(filename: &str) -> String {
    Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .unwrap_or("download")
        .to_owned()
}

fn download_files(history: &DownloadHistory) -> Vec<PathBuf> {
    let mut files = history.borrow().clone();
    if let Ok(entries) = std::fs::read_dir(default_download_directory()) {
        files.extend(
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_file()),
        );
    }
    files.sort();
    files.dedup();
    files
}

pub(crate) fn show_downloads_dialog(window: &ApplicationWindow, history: DownloadHistory) {
    let dialog = Dialog::with_buttons(
        Some("Downloads"),
        Some(window),
        DialogFlags::MODAL | DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Abrir pasta", ResponseType::Accept),
            ("Fechar", ResponseType::Cancel),
        ],
    );
    dialog.set_default_size(620, 420);

    let content = dialog.content_area();
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    let list = ListBox::new();
    for path in download_files(&history) {
        let row = ListBoxRow::new();
        let label = Label::new(Some(&path.to_string_lossy()));
        label.set_xalign(0.0);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        label.set_margin_start(8);
        label.set_margin_end(8);
        row.add(&label);
        list.add(&row);
    }
    if list.children().is_empty() {
        let empty = Label::new(Some("Nenhum arquivo baixado ainda."));
        empty.set_xalign(0.0);
        list.add(&empty);
    }
    let scroll = ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.set_min_content_height(280);
    scroll.set_vexpand(true);
    scroll.add(&list);
    content.pack_start(&scroll, true, true, 0);

    let downloads_directory = default_download_directory();
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            let _ = std::fs::create_dir_all(&downloads_directory);
            let _ = Command::new("xdg-open").arg(&downloads_directory).spawn();
        }
        dialog.close();
    });
    dialog.show_all();
}

fn set_download_button_icon(button: &Button, icon_name: &str) {
    let image = Image::from_icon_name(Some(icon_name), IconSize::Button);
    button.set_image(Some(&image));
}

fn animate_download_button(button: Button, mut child: Child) {
    let alternate = Rc::new(Cell::new(false));
    button.set_tooltip_text(Some("Download em andamento"));
    glib::timeout_add_local(Duration::from_millis(180), move || match child.try_wait() {
        Ok(Some(_)) | Err(_) => {
            set_download_button_icon(&button, "folder-download");
            button.set_tooltip_text(Some("Mostrar arquivos baixados"));
            glib::ControlFlow::Break
        }
        Ok(None) => {
            let next = !alternate.get();
            alternate.set(next);
            set_download_button_icon(
                &button,
                if next {
                    "view-refresh"
                } else {
                    "folder-download"
                },
            );
            glib::ControlFlow::Continue
        }
    });
}

fn start_external_download(
    source_url: &str,
    destination: &Path,
    status: &Label,
    downloads_button: Option<&Button>,
    history: &DownloadHistory,
) {
    let destination_path = destination.to_path_buf();
    let destination = destination.to_string_lossy().into_owned();
    let curl_result = Command::new("curl")
        .args(["--fail", "--location", "--output"])
        .arg(&destination)
        .arg(source_url)
        .spawn();

    let result = curl_result.or_else(|_| {
        Command::new("wget")
            .args(["--continue", "--output-document"])
            .arg(&destination)
            .arg(source_url)
            .spawn()
    });

    match result {
        Ok(child) => {
            {
                let mut history_ref = history.borrow_mut();
                if !history_ref.contains(&destination_path) {
                    history_ref.push(destination_path);
                }
            }
            let history_save_result = save_download_history(history);
            status.set_text(&format!("Download iniciado em {destination}"));
            if let Some(button) = downloads_button {
                animate_download_button(button.clone(), child);
            }
            if let Err(error) = history_save_result {
                status.set_text(&format!(
                    "Download iniciado, mas histórico não salvo: {error}"
                ));
            }
        }
        Err(error) => status.set_text(&format!(
            "Não foi possível iniciar o download (curl/wget): {error}"
        )),
    }
}

pub(crate) fn configure_download_policy(
    context: &WebContext,
    window: &ApplicationWindow,
    status: Label,
    downloads_button: Option<Button>,
    download_history: DownloadHistory,
) {
    let window = window.clone();
    context.connect_download_started(move |_context, download| {
        let window_for_download = window.clone();
        let status_for_download = status.clone();
        let downloads_button_for_download = downloads_button.clone();
        let download_history_for_download = Rc::clone(&download_history);
        let source_url = download
            .request()
            .and_then(|request| request.uri())
            .map(|uri| uri.to_string())
            .or_else(|| {
                download
                    .response()
                    .and_then(|response| response.uri())
                    .map(|uri| uri.to_string())
            });
        let suggested_filename = download
            .response()
            .and_then(|response| response.suggested_filename())
            .map(|filename| safe_download_filename(filename.as_str()));

        // Do not keep this WebKit download alive while a GTK dialog is open.
        // In particular, ISO responses can crash WebKitGTK when a destination
        // is assigned asynchronously after the native download has started.
        download.cancel();

        let Some(source_url) = source_url else {
            status_for_download.set_text("Download recusado: URL indisponível");
            return;
        };
        let suggested_filename =
            suggested_filename.unwrap_or_else(|| suggested_filename_from_url(&source_url));
        request_external_download(
            &window_for_download,
            status_for_download,
            downloads_button_for_download,
            download_history_for_download,
            source_url,
            suggested_filename,
        );
    });
}

pub(crate) fn request_external_download(
    window: &ApplicationWindow,
    status: Label,
    downloads_button: Option<Button>,
    download_history: DownloadHistory,
    source_url: String,
    suggested_filename: String,
) {
    status.set_text("Download aguardando autorização");
    let window = window.clone();
    glib::idle_add_local_once(move || {
        let approval = MessageDialog::new(
            Some(&window),
            DialogFlags::MODAL | DialogFlags::DESTROY_WITH_PARENT,
            MessageType::Question,
            ButtonsType::None,
            "Permitir este download?",
        );
        approval.set_title("Download solicitado");
        approval.set_secondary_text(Some(
            "Uma página solicitou um arquivo. Permita o download para escolher onde salvá-lo.",
        ));
        approval.add_button("Bloquear", ResponseType::Cancel);
        approval.add_button("Permitir", ResponseType::Accept);
        approval.connect_response(move |dialog, response| {
            if response != ResponseType::Accept {
                dialog.close();
                dialog.hide();
                status.set_text("Download bloqueado");
                return;
            }

            let window_for_destination = window.clone();
            let source_url_for_destination = source_url.clone();
            let suggested_filename_for_destination = suggested_filename.clone();
            let status_for_destination = status.clone();
            let downloads_button_for_destination = downloads_button.clone();
            let download_history_for_destination = Rc::clone(&download_history);
            dialog.close();
            dialog.hide();
            status.set_text("Download autorizado; escolhendo destino");
            glib::idle_add_local_once(move || {
                show_download_destination_chooser(
                    &window_for_destination,
                    &source_url_for_destination,
                    &suggested_filename_for_destination,
                    &status_for_destination,
                    downloads_button_for_destination,
                    download_history_for_destination,
                );
            });
        });
        approval.show_all();
    });
}

fn percent_encode_query(query: &str) -> String {
    let mut encoded = String::with_capacity(query.len());
    for byte in query.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(*byte as char);
        } else {
            encoded.push('%');
            encoded.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            encoded.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn show_http_confirmation(
    window: &ApplicationWindow,
    shell: Rc<RefCell<BrowserShell>>,
    engine: Rc<RefCell<GtkEngine>>,
    tab_id: TabId,
    status: Label,
) {
    let dialog = MessageDialog::new(
        Some(window),
        DialogFlags::MODAL,
        MessageType::Warning,
        ButtonsType::None,
        "Esta página usa HTTP sem criptografia. Deseja continuar?",
    );
    dialog.add_button("Cancelar", ResponseType::Cancel);
    dialog.add_button("Continuar", ResponseType::Accept);
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            match shell
                .borrow_mut()
                .confirm_insecure_navigation_with_engine(&mut *engine.borrow_mut(), tab_id)
            {
                Ok(_) => status.set_text("Navegação HTTP iniciada"),
                Err(error) => status.set_text(&shell_error_text(error)),
            }
        } else {
            let _ = shell.borrow_mut().cancel_insecure_navigation(tab_id);
            status.set_text("Navegação cancelada");
        }
        dialog.close();
    });
    dialog.show_all();
}

fn shell_error_text(error: ShellError) -> String {
    match error {
        ShellError::UnknownTab(_) => "Aba inexistente".to_owned(),
        ShellError::NoPendingNavigation(_) => "Nenhuma confirmação pendente".to_owned(),
        ShellError::InvalidUrl(_) => "URL inválida ou esquema não permitido".to_owned(),
        ShellError::Permission(_) => "Permissão recusada".to_owned(),
        ShellError::Engine(_) => "A engine recusou a navegação".to_owned(),
        ShellError::InvalidEngineEvent(_) => "Evento inválido da engine".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buscador_codifica_consulta_sem_perder_unicode() {
        assert_eq!(
            SearchEngine::DuckDuckGo.search_url("privacidade e segurança"),
            "https://duckduckgo.com/?q=privacidade%20e%20seguran%C3%A7a"
        );
    }

    #[test]
    fn cada_buscador_tem_endpoint_explicito() {
        assert!(SearchEngine::Brave
            .search_url("aegis")
            .starts_with("https://search.brave.com/search?q="));
        assert!(SearchEngine::Google
            .search_url("aegis")
            .starts_with("https://www.google.com/search?q="));
        assert!(SearchEngine::Startpage
            .search_url("aegis")
            .starts_with("https://www.startpage.com/sp/search?query="));
        assert!(SearchEngine::Qwant
            .search_url("aegis")
            .starts_with("https://www.qwant.com/?q="));
        assert!(SearchEngine::Bing
            .search_url("aegis")
            .starts_with("https://www.bing.com/search?q="));
    }

    #[test]
    fn preferencias_serializadas_incluem_todas_as_opcoes_do_painel() {
        let stored = StoredPreferences {
            search_engine: Some("google".to_owned()),
            theme: Some("dark".to_owned()),
            show_bookmarks_bar: Some(false),
            active_profile: Some("Trabalho".to_owned()),
            profiles: Some(vec!["Pessoal".to_owned(), "Trabalho".to_owned()]),
            https_only: Some(false),
            allow_loopback_http: Some(true),
            confirm_http_exceptions: Some(false),
        };
        let encoded = serde_json::to_string(&stored).unwrap();
        let decoded: StoredPreferences = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.search_engine.as_deref(), Some("google"));
        assert_eq!(decoded.theme.as_deref(), Some("dark"));
        assert_eq!(decoded.show_bookmarks_bar, Some(false));
        assert_eq!(decoded.active_profile.as_deref(), Some("Trabalho"));
        assert_eq!(decoded.profiles.as_ref().map(Vec::len), Some(2));
        assert_eq!(decoded.https_only, Some(false));
        assert_eq!(decoded.allow_loopback_http, Some(true));
        assert_eq!(decoded.confirm_http_exceptions, Some(false));
    }
}
