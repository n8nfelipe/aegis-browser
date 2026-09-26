use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    Image, Label, MessageDialog, MessageType, Notebook, Orientation, PositionType, ProgressBar,
    ResponseType, Window, WindowType,
};
use webkit2gtk::{
    DownloadExt, HardwareAccelerationPolicy, LoadEvent, NavigationPolicyDecision,
    NavigationPolicyDecisionExt, PolicyDecisionExt, PolicyDecisionType, Settings, SettingsExt,
    URIRequestExt, WebContext, WebContextExt, WebView, WebViewExt,
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
        match id.as_deref() {
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
        match id.as_deref() {
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
}

impl Default for BrowserPreferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            search_engine: SearchEngine::DuckDuckGo,
            show_bookmarks_bar: true,
            active_profile: "Pessoal".to_owned(),
            profiles: vec!["Pessoal".to_owned()],
        }
    }
}

pub(crate) type ProfileContexts = Rc<RefCell<HashMap<String, WebContext>>>;
pub(crate) type FaviconCacheDirs = Rc<RefCell<Vec<PathBuf>>>;

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

pub(crate) fn new_ephemeral_context(favicon_cache_dirs: &FaviconCacheDirs) -> WebContext {
    let context = WebContext::new_ephemeral();
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

const APPLICATION_ID: &str = "org.aegis.Browser";

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
    let context = new_ephemeral_context(&favicon_cache_dirs);
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

    let status_for_download = status.clone();
    context.connect_download_started(move |_context, download| {
        // No destination is configured, and cancellation happens immediately
        // so a site cannot silently place a file on disk.
        download.cancel();
        status_for_download.set_text("Download bloqueado por padrão");
    });

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

    let downloads = Label::new(Some("Downloads: bloqueados por padrão."));
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
                    let context = new_ephemeral_context(&favicon_cache_dirs_for_response);
                    configure_download_policy(&context, status.clone());
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
            };
            if let Some(bookmarks_bar) = bookmarks_bar_for_response.as_ref() {
                bookmarks_bar.set_visible(show_bookmarks_bar);
            }
            apply_theme(selected_theme);
            status.set_text(&format!(
                "Configurações aplicadas — novas abas: {selected_profile}"
            ));
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

pub(crate) fn configure_download_policy(context: &WebContext, status: Label) {
    context.connect_download_started(move |_context, download| {
        download.cancel();
        status.set_text("Download bloqueado por padrão");
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
}
