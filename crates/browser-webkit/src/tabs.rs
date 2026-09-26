use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use aegis_browser_shell::{
    BrowserShell, EngineEvent, LoadError, NavigationResult, TabId, TabStatus, TitleChanged,
};
use aegis_policy_core::{BrowserPolicy, NavigationDecision, NavigationRequest};
use aegis_url_adapter::WebUrl;
use gtk::cairo::{Context, Format, ImageSurface, Surface};
use gtk::gdk_pixbuf::InterpType;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, ButtonsType, Dialog, DialogFlags, Entry,
    IconSize, Image, Label, MessageDialog, MessageType, Notebook, Orientation, PositionType,
    ProgressBar, ResponseType, ScrolledWindow,
};
#[allow(deprecated)]
use webkit2gtk::InstallMissingMediaPluginsPermissionRequest;
use webkit2gtk::{
    HardwareAccelerationPolicy, LoadEvent, NavigationPolicyDecision, NavigationPolicyDecisionExt,
    PermissionRequestExt, PolicyDecisionExt, PolicyDecisionType, Settings, SettingsExt,
    URIRequestExt, UserContentInjectedFrames, UserContentManager, UserContentManagerExt,
    UserScript, UserScriptInjectionTime, WebContext, WebView, WebViewExt,
};

use super::{
    configure_download_policy, new_profile_context, setup_favicon_cache_cleanup, shell_error_text,
    show_http_confirmation, show_settings_dialog, BrowserPreferences, GtkEngine, ProfileContexts,
};

const TAB_FAVICON_SIZE: i32 = 16;
const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/aegis-browser.png");

#[derive(Clone, Copy)]
enum SecurityIcon {
    Neutral,
    Secure,
    Warning,
}

#[derive(Clone)]
struct TabHandle {
    webview: WebView,
    engine: Rc<RefCell<GtkEngine>>,
}

pub(super) fn build_ui(application: &Application) {
    let policy_state = Rc::new(RefCell::new(BrowserPolicy::default()));
    let shell = Rc::new(RefCell::new(BrowserShell::new(*policy_state.borrow())));
    let preferences = Rc::new(RefCell::new(BrowserPreferences::default()));
    let favicon_cache_dirs = setup_favicon_cache_cleanup(application);
    let context = new_profile_context("Pessoal", &favicon_cache_dirs);
    let profile_contexts: ProfileContexts = Rc::new(RefCell::new(HashMap::from([(
        "Pessoal".to_owned(),
        context.clone(),
    )])));
    let tabs: Rc<RefCell<HashMap<TabId, TabHandle>>> = Rc::new(RefCell::new(HashMap::new()));
    let page_tabs = Rc::new(RefCell::new(Vec::<TabId>::new()));

    let window = ApplicationWindow::new(application);
    window.set_title("Aegis Browser — WebKitGTK");
    set_window_icon(&window);
    window.set_default_size(1200, 800);

    let root = GtkBox::new(Orientation::Vertical, 0);
    let toolbar = GtkBox::new(Orientation::Horizontal, 6);
    toolbar.set_margin_top(8);
    toolbar.set_margin_bottom(8);
    toolbar.set_margin_start(8);
    toolbar.set_margin_end(8);

    let security = create_security_icon(SecurityIcon::Neutral);
    security.set_pixel_size(20);
    security.set_tooltip_text(Some("Nenhuma conexão ativa"));
    let address = Entry::new();
    address.set_placeholder_text(Some("Digite uma URL HTTPS"));
    address.set_hexpand(true);
    let go = Button::with_label("Ir");
    let back = Button::with_label("←");
    let forward = Button::with_label("→");
    let reload = Button::from_icon_name(Some("view-refresh"), IconSize::Button);
    back.set_sensitive(false);
    forward.set_sensitive(false);
    back.set_tooltip_text(Some("Voltar"));
    forward.set_tooltip_text(Some("Avançar"));
    reload.set_tooltip_text(Some("Atualizar página (Ctrl+R ou F5)"));
    let new_tab = Button::with_label("Nova aba");
    let settings = Button::with_label("Configurações");
    let add_bookmark = Button::with_label("☆");
    add_bookmark.set_tooltip_text(Some("Adicionar página aos favoritos"));
    let bitwarden_button = Button::with_label("Bitwarden");
    bitwarden_button.set_tooltip_text(Some("Preencher credencial com Bitwarden"));
    let extension_toolbar = GtkBox::new(Orientation::Horizontal, 2);
    let bookmarks_bar = GtkBox::new(Orientation::Horizontal, 6);
    bookmarks_bar.set_margin_start(12);
    bookmarks_bar.set_margin_end(12);
    bookmarks_bar.set_margin_bottom(6);
    bookmarks_bar.set_visible(preferences.borrow().show_bookmarks_bar);
    let bookmarks: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
    let status = Label::new(Some("Pronto"));
    let progress = ProgressBar::new();
    progress.set_show_text(false);

    if let Some(icon) =
        load_app_icon().and_then(|icon| icon.scale_simple(24, 24, InterpType::Bilinear))
    {
        let brand_icon = Image::from_pixbuf(Some(&icon));
        brand_icon.set_size_request(24, 24);
        brand_icon.set_tooltip_text(Some("Aegis Browser"));
        toolbar.pack_start(&brand_icon, false, false, 0);
    }
    toolbar.pack_start(&security, false, false, 0);
    toolbar.pack_start(&back, false, false, 0);
    toolbar.pack_start(&forward, false, false, 0);
    toolbar.pack_start(&reload, false, false, 0);
    toolbar.pack_start(&address, true, true, 0);
    toolbar.pack_start(&go, false, false, 0);
    toolbar.pack_start(&add_bookmark, false, false, 0);
    toolbar.pack_start(&bitwarden_button, false, false, 0);
    toolbar.pack_start(&new_tab, false, false, 0);
    toolbar.pack_start(&settings, false, false, 0);
    toolbar.pack_start(&extension_toolbar, false, false, 0);
    toolbar.pack_start(&status, false, false, 0);
    root.pack_start(&toolbar, false, false, 0);
    root.pack_start(&progress, false, false, 0);
    root.pack_start(&bookmarks_bar, false, false, 0);

    let notebook = Notebook::new();
    notebook.set_show_tabs(true);
    notebook.set_tab_pos(PositionType::Top);
    notebook.set_scrollable(true);
    root.pack_start(&notebook, true, true, 0);
    window.add(&root);
    super::refresh_extensions_toolbar(&extension_toolbar);

    configure_download_policy(&context, status.clone());

    let shell_for_new_tab = Rc::clone(&shell);
    let notebook_for_new_tab = notebook.clone();
    let page_tabs_for_new_tab = Rc::clone(&page_tabs);
    let tabs_for_new_tab = Rc::clone(&tabs);
    let default_context_for_new_tab = context.clone();
    let policy_for_new_tab = Rc::clone(&policy_state);
    let preferences_for_new_tab = Rc::clone(&preferences);
    let profile_contexts_for_new_tab = Rc::clone(&profile_contexts);
    let status_for_new_tab = status.clone();
    let progress_for_new_tab = progress.clone();
    let back_for_new_tab = back.clone();
    let forward_for_new_tab = forward.clone();
    let security_for_new_tab = security.clone();
    new_tab.connect_clicked(move |_| {
        let active_profile = preferences_for_new_tab.borrow().active_profile.clone();
        let context_for_new_tab = profile_contexts_for_new_tab
            .borrow()
            .get(&active_profile)
            .cloned()
            .unwrap_or_else(|| default_context_for_new_tab.clone());
        let tab_id = add_tab(
            Rc::clone(&shell_for_new_tab),
            &notebook_for_new_tab,
            Rc::clone(&page_tabs_for_new_tab),
            Rc::clone(&tabs_for_new_tab),
            &context_for_new_tab,
            Rc::clone(&policy_for_new_tab),
            status_for_new_tab.clone(),
            progress_for_new_tab.clone(),
            back_for_new_tab.clone(),
            forward_for_new_tab.clone(),
            security_for_new_tab.clone(),
        );
        if let Some(page) = page_tabs_for_new_tab
            .borrow()
            .iter()
            .position(|id| *id == tab_id)
        {
            notebook_for_new_tab.set_current_page(Some(page as u32));
        }
        status_for_new_tab.set_text("Nova aba criada");
    });

    let initial_tab = add_tab(
        Rc::clone(&shell),
        &notebook,
        Rc::clone(&page_tabs),
        Rc::clone(&tabs),
        &context,
        Rc::clone(&policy_state),
        status.clone(),
        progress.clone(),
        back.clone(),
        forward.clone(),
        security.clone(),
    );

    let shell_for_switch = Rc::clone(&shell);
    let page_tabs_for_switch = Rc::clone(&page_tabs);
    let tabs_for_switch = Rc::clone(&tabs);
    let address_for_switch = address.clone();
    let security_for_switch = security.clone();
    let status_for_switch = status.clone();
    let back_for_switch = back.clone();
    let forward_for_switch = forward.clone();
    notebook.connect_switch_page(move |_notebook, _page, page_num| {
        let Some(tab_id) = page_tabs_for_switch
            .borrow()
            .get(page_num as usize)
            .copied()
        else {
            return;
        };
        let _ = shell_for_switch.borrow_mut().select_tab(tab_id);
        sync_active_tab(
            &shell_for_switch,
            tab_id,
            &address_for_switch,
            &security_for_switch,
            &status_for_switch,
        );
        sync_navigation_buttons(
            tab_id,
            &tabs_for_switch,
            &back_for_switch,
            &forward_for_switch,
        );
    });
    notebook.set_current_page(Some(0));
    sync_active_tab(&shell, initial_tab, &address, &security, &status);
    sync_navigation_buttons(initial_tab, &tabs, &back, &forward);

    let reload_current: Rc<dyn Fn()> = Rc::new({
        let notebook = notebook.clone();
        let page_tabs = Rc::clone(&page_tabs);
        let tabs = Rc::clone(&tabs);
        let status = status.clone();
        move || {
            let Some(page) = notebook.current_page() else {
                status.set_text("Nenhuma aba ativa");
                return;
            };
            let Some(tab_id) = page_tabs.borrow().get(page as usize).copied() else {
                status.set_text("Aba inválida");
                return;
            };
            let Some(tab) = tabs.borrow().get(&tab_id).cloned() else {
                status.set_text("Aba inexistente");
                return;
            };
            tab.webview.reload();
            status.set_text("Atualizando");
        }
    });
    let reload_for_button = Rc::clone(&reload_current);
    reload.connect_clicked(move |_| reload_for_button());
    let reload_for_keys = Rc::clone(&reload_current);
    window.connect_key_press_event(move |_window, event| {
        let key = event.keyval();
        let ctrl_reload = event.state().contains(gtk::gdk::ModifierType::CONTROL_MASK)
            && (key == gtk::gdk::keys::constants::R || key == gtk::gdk::keys::constants::r);
        if ctrl_reload || key == gtk::gdk::keys::constants::F5 {
            reload_for_keys();
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });

    let notebook_for_back = notebook.clone();
    let page_tabs_for_back = Rc::clone(&page_tabs);
    let tabs_for_back = Rc::clone(&tabs);
    back.connect_clicked(move |_| {
        let Some(page) = notebook_for_back.current_page() else {
            return;
        };
        let Some(tab_id) = page_tabs_for_back.borrow().get(page as usize).copied() else {
            return;
        };
        if let Some(tab) = tabs_for_back.borrow().get(&tab_id).cloned() {
            tab.webview.go_back();
        }
    });

    let notebook_for_forward = notebook.clone();
    let page_tabs_for_forward = Rc::clone(&page_tabs);
    let tabs_for_forward = Rc::clone(&tabs);
    forward.connect_clicked(move |_| {
        let Some(page) = notebook_for_forward.current_page() else {
            return;
        };
        let Some(tab_id) = page_tabs_for_forward.borrow().get(page as usize).copied() else {
            return;
        };
        if let Some(tab) = tabs_for_forward.borrow().get(&tab_id).cloned() {
            tab.webview.go_forward();
        }
    });

    let window_for_settings = window.clone();
    let shell_for_settings = Rc::clone(&shell);
    let policy_for_settings = Rc::clone(&policy_state);
    let preferences_for_settings = Rc::clone(&preferences);
    let profile_contexts_for_settings = Rc::clone(&profile_contexts);
    let favicon_cache_dirs_for_settings = Rc::clone(&favicon_cache_dirs);
    let status_for_settings = status.clone();
    let extension_toolbar_for_settings = extension_toolbar.clone();
    let bookmarks_bar_for_settings = bookmarks_bar.clone();
    settings.connect_clicked(move |_| {
        show_settings_dialog(
            &window_for_settings,
            Rc::clone(&shell_for_settings),
            Rc::clone(&policy_for_settings),
            Rc::clone(&preferences_for_settings),
            Rc::clone(&profile_contexts_for_settings),
            Rc::clone(&favicon_cache_dirs_for_settings),
            status_for_settings.clone(),
            Some(extension_toolbar_for_settings.clone()),
            Some(bookmarks_bar_for_settings.clone()),
        );
    });

    let window_for_bitwarden = window.clone();
    let notebook_for_bitwarden = notebook.clone();
    let page_tabs_for_bitwarden = Rc::clone(&page_tabs);
    let tabs_for_bitwarden = Rc::clone(&tabs);
    let address_for_bitwarden = address.clone();
    let status_for_bitwarden = status.clone();
    bitwarden_button.connect_clicked(move |_| {
        let Some(page) = notebook_for_bitwarden.current_page() else {
            status_for_bitwarden.set_text("Nenhuma aba ativa");
            return;
        };
        let Some(tab_id) = page_tabs_for_bitwarden.borrow().get(page as usize).copied() else {
            status_for_bitwarden.set_text("Aba inválida");
            return;
        };
        let Some(tab) = tabs_for_bitwarden.borrow().get(&tab_id).cloned() else {
            status_for_bitwarden.set_text("Aba inexistente");
            return;
        };
        let current_url = tab
            .webview
            .uri()
            .map(|uri| uri.to_string())
            .unwrap_or_else(|| address_for_bitwarden.text().trim().to_owned());
        let Ok(url) = WebUrl::parse(&current_url) else {
            show_bitwarden_error(
                &window_for_bitwarden,
                "Abra uma página HTTP ou HTTPS para usar o Bitwarden.",
            );
            return;
        };
        match super::bitwarden::list_logins(&url.origin().canonical_string()) {
            Ok(logins) => show_bitwarden_dialog(
                &window_for_bitwarden,
                &tab.webview,
                logins,
                status_for_bitwarden.clone(),
            ),
            Err(error) => show_bitwarden_error(&window_for_bitwarden, &error),
        }
    });

    let navigate: Rc<dyn Fn()> = Rc::new({
        let shell = Rc::clone(&shell);
        let notebook = notebook.clone();
        let page_tabs = Rc::clone(&page_tabs);
        let tabs = Rc::clone(&tabs);
        let address = address.clone();
        let security = security.clone();
        let status = status.clone();
        let window = window.clone();
        let preferences = Rc::clone(&preferences);
        move || {
            let Some(page) = notebook.current_page() else {
                status.set_text("Nenhuma aba ativa");
                return;
            };
            let Some(tab_id) = page_tabs.borrow().get(page as usize).copied() else {
                status.set_text("Aba inválida");
                return;
            };
            let Some(tab) = tabs.borrow().get(&tab_id).cloned() else {
                status.set_text("Aba inexistente");
                return;
            };
            let input = address.text().trim().to_owned();
            if input.is_empty() {
                status.set_text("Digite um endereço");
                return;
            }
            let destination = resolve_navigation_input(&input, preferences.borrow().search_engine);

            let result = shell.borrow_mut().navigate_with_engine(
                &mut *tab.engine.borrow_mut(),
                tab_id,
                &destination,
                true,
            );
            match result {
                Ok(NavigationResult::Navigated { .. }) => {
                    if let Ok(url) = WebUrl::parse(&destination) {
                        set_security_indicator(&security, Some(&url));
                    }
                    status.set_text("Carregando");
                }
                Ok(NavigationResult::BlockedInsecure { .. }) => {
                    set_security_icon(&security, SecurityIcon::Warning);
                    security.set_tooltip_text(Some("Navegação HTTP bloqueada"));
                    status.set_text("Navegação insegura bloqueada");
                }
                Ok(NavigationResult::ConfirmationRequired { tab_id, .. }) => {
                    show_http_confirmation(
                        &window,
                        Rc::clone(&shell),
                        Rc::clone(&tab.engine),
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
    let navigate_for_address = Rc::clone(&navigate);
    address.connect_activate(move |_| navigate_for_address());

    let bookmarks_for_add = Rc::clone(&bookmarks);
    let bookmarks_bar_for_add = bookmarks_bar.clone();
    let address_for_bookmark = address.clone();
    let navigate_for_bookmark = Rc::clone(&navigate);
    add_bookmark.connect_clicked(move |_| {
        let url = address_for_bookmark.text().trim().to_owned();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return;
        }
        let mut bookmarks = bookmarks_for_add.borrow_mut();
        if !bookmarks.iter().any(|(_, saved_url)| saved_url == &url) {
            bookmarks.push((url.clone(), url));
        }
        drop(bookmarks);
        refresh_bookmarks_bar(
            &bookmarks_bar_for_add,
            &bookmarks_for_add,
            &address_for_bookmark,
            &navigate_for_bookmark,
        );
    });
    refresh_bookmarks_bar(&bookmarks_bar, &bookmarks, &address, &navigate);

    window.show_all();
}

fn resolve_navigation_input(input: &str, search_engine: super::SearchEngine) -> String {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();
    let has_explicit_scheme = lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("javascript:")
        || lower.starts_with("file:")
        || lower.starts_with("data:")
        || lower.starts_with("about:")
        || lower.starts_with("view-source:");
    let looks_like_host = trimmed.contains('.')
        || trimmed.contains('/')
        || trimmed.contains(':')
        || lower.starts_with("localhost")
        || lower.starts_with("[");

    if has_explicit_scheme || (looks_like_host && WebUrl::parse_user_input(trimmed).is_ok()) {
        trimmed.to_owned()
    } else {
        search_engine.search_url(trimmed)
    }
}

#[allow(clippy::too_many_arguments)]
fn add_tab(
    shell: Rc<RefCell<BrowserShell>>,
    notebook: &Notebook,
    page_tabs: Rc<RefCell<Vec<TabId>>>,
    tabs: Rc<RefCell<HashMap<TabId, TabHandle>>>,
    context: &WebContext,
    policy_state: Rc<RefCell<BrowserPolicy>>,
    status: Label,
    progress: ProgressBar,
    back_button: Button,
    forward_button: Button,
    security: Image,
) -> TabId {
    let tab_id = shell.borrow_mut().new_tab();
    let user_content_manager = UserContentManager::new();
    user_content_manager.add_script(&UserScript::new(
        AVIF_FALLBACK_SCRIPT,
        UserContentInjectedFrames::TopFrame,
        UserScriptInjectionTime::End,
        &[],
        &[],
    ));
    let loaded_extensions = super::extensions::load_content_scripts(&user_content_manager);
    if loaded_extensions > 0 {
        eprintln!("Aegis: {loaded_extensions} recurso(s) de extensão ativo(s) nesta aba");
    }
    let webview = WebView::builder()
        .web_context(context)
        .user_content_manager(&user_content_manager)
        .build();
    let settings = Settings::new();
    settings.set_enable_developer_extras(false);
    settings.set_enable_javascript(true);
    settings.set_user_agent(Some(super::COMPATIBLE_USER_AGENT));
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
    // Keep autoplay conservative: a page must receive a user gesture before
    // starting media, while ordinary HTML5 video controls remain available.
    settings.set_media_playback_requires_user_gesture(true);
    settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::OnDemand);
    webview.set_settings(&settings);

    let approved_loads = Rc::new(RefCell::new(HashSet::new()));
    connect_navigation_policy(&webview, policy_state, Rc::clone(&approved_loads));
    connect_media_permission_policy(&webview, Rc::clone(&shell), tab_id, status.clone());
    let engine = Rc::new(RefCell::new(GtkEngine {
        webview: webview.clone(),
        approved_loads,
    }));

    let favicon = Image::from_icon_name(Some("web-browser"), IconSize::Menu);
    favicon.set_pixel_size(TAB_FAVICON_SIZE);
    favicon.set_size_request(TAB_FAVICON_SIZE, TAB_FAVICON_SIZE);
    let title_label = Label::new(Some("Nova aba"));
    let close_button = Button::with_label("×");
    close_button.set_focus_on_click(false);
    let tab_label = GtkBox::new(Orientation::Horizontal, 4);
    tab_label.pack_start(&favicon, false, false, 0);
    tab_label.pack_start(&title_label, true, true, 0);
    tab_label.pack_start(&close_button, false, false, 0);
    tab_label.show_all();
    notebook.append_page(&webview, Some(&tab_label));
    // Pages added after the window is already visible need an explicit show;
    // otherwise GTK can keep rendering the previously selected WebView.
    webview.show();
    notebook.show_all();
    // The page-to-TabId map is intentionally stable until drag-reordering is
    // implemented together with a `page-reordered` synchronization path.
    notebook.set_tab_reorderable(&webview, false);

    tabs.borrow_mut().insert(
        tab_id,
        TabHandle {
            webview: webview.clone(),
            engine,
        },
    );
    page_tabs.borrow_mut().push(tab_id);

    let notebook_for_close = notebook.clone();
    let page_tabs_for_close = Rc::clone(&page_tabs);
    let tabs_for_close = Rc::clone(&tabs);
    let shell_for_close = Rc::clone(&shell);
    let status_for_close = status.clone();
    close_button.connect_clicked(move |_| {
        close_tab(
            &notebook_for_close,
            &page_tabs_for_close,
            &tabs_for_close,
            &shell_for_close,
            tab_id,
            &status_for_close,
        );
    });

    let favicon_for_notify = favicon.clone();
    webview.connect_favicon_notify(move |view| {
        refresh_tab_favicon(view, &favicon_for_notify);
    });

    let shell_for_events = Rc::clone(&shell);
    let status_for_events = status.clone();
    let progress_for_events = progress.clone();
    let favicon_for_load = favicon.clone();
    let security_for_events = security.clone();
    let back_for_events = back_button;
    let forward_for_events = forward_button;
    webview.connect_load_changed(move |view, event| {
        match event {
            // Do not keep showing the previous page's icon while navigating.
            LoadEvent::Started => reset_tab_favicon(&favicon_for_load),
            // Some pages publish their favicon only after the document has
            // finished loading; refresh once more in addition to notify::favicon.
            LoadEvent::Finished => refresh_tab_favicon(view, &favicon_for_load),
            _ => {}
        }
        let engine_event = match event {
            LoadEvent::Started => Some(EngineEvent::LoadStarted { tab_id }),
            LoadEvent::Finished => Some(EngineEvent::LoadFinished { tab_id }),
            _ => None,
        };
        if let Some(engine_event) = engine_event {
            let invalid = shell_for_events
                .borrow_mut()
                .handle_engine_event(engine_event)
                .is_err();
            let active = shell_for_events.borrow().active_tab_id() == Some(tab_id);
            if active && invalid {
                status_for_events.set_text("Evento inválido da engine");
            } else if active && event == LoadEvent::Finished {
                progress_for_events.set_fraction(1.0);
                status_for_events.set_text("Carregado");
            } else if active {
                progress_for_events.set_fraction(0.0);
                status_for_events.set_text("Carregando");
            }
        }
        if shell_for_events.borrow().active_tab_id() == Some(tab_id) {
            if let Some(uri) = view.uri() {
                if let Ok(url) = WebUrl::parse(uri.as_str()) {
                    set_security_indicator(&security_for_events, Some(&url));
                }
            }
            back_for_events.set_sensitive(view.can_go_back());
            forward_for_events.set_sensitive(view.can_go_forward());
        }
    });

    let shell_for_title = Rc::clone(&shell);
    let status_for_title = status.clone();
    let title_label_for_title = title_label.clone();
    webview.connect_title_notify(move |view| {
        if let Some(title) = view.title() {
            let result = shell_for_title
                .borrow_mut()
                .handle_title_changed(TitleChanged {
                    tab_id,
                    title: title.to_string(),
                });
            if result.is_err() {
                if shell_for_title.borrow().active_tab_id() == Some(tab_id) {
                    status_for_title.set_text("Título inválido");
                }
            } else {
                title_label_for_title.set_text(&tab_display_name(&shell_for_title, tab_id));
            }
        }
    });

    let shell_for_progress = Rc::clone(&shell);
    webview.connect_estimated_load_progress_notify(move |view| {
        if shell_for_progress.borrow().active_tab_id() == Some(tab_id) {
            progress.set_fraction(view.estimated_load_progress());
        }
    });

    let shell_for_failure = Rc::clone(&shell);
    webview.connect_load_failed(move |_view, _event, _failing_uri, _error| {
        let result = shell_for_failure
            .borrow_mut()
            .handle_engine_event(EngineEvent::LoadFailed {
                tab_id,
                error: LoadError::Unknown,
            });
        if shell_for_failure.borrow().active_tab_id() == Some(tab_id) {
            if result.is_err() {
                status.set_text("Falha de carregamento inválida");
            } else {
                status.set_text("Falha ao carregar");
            }
        }
        false
    });

    tab_id
}

fn reset_tab_favicon(image: &Image) {
    image.set_from_icon_name(Some("web-browser"), IconSize::Menu);
    image.set_pixel_size(TAB_FAVICON_SIZE);
    image.set_size_request(TAB_FAVICON_SIZE, TAB_FAVICON_SIZE);
}

fn refresh_tab_favicon(view: &WebView, image: &Image) {
    if let Some(surface) = view.favicon() {
        if let Some(scaled_surface) = scale_favicon_surface(&surface) {
            image.set_from_surface(Some(&scaled_surface));
            image.set_size_request(TAB_FAVICON_SIZE, TAB_FAVICON_SIZE);
        } else {
            reset_tab_favicon(image);
        }
    } else {
        reset_tab_favicon(image);
    }
}

fn scale_favicon_surface(surface: &Surface) -> Option<ImageSurface> {
    let source = ImageSurface::try_from(surface.clone()).ok()?;
    let source_width = source.width();
    let source_height = source.height();
    if source_width <= 0 || source_height <= 0 {
        return None;
    }

    let scale = (TAB_FAVICON_SIZE as f64 / source_width as f64)
        .min(TAB_FAVICON_SIZE as f64 / source_height as f64)
        .min(1.0);
    let rendered_width = source_width as f64 * scale;
    let rendered_height = source_height as f64 * scale;
    let offset_x = (TAB_FAVICON_SIZE as f64 - rendered_width) / 2.0;
    let offset_y = (TAB_FAVICON_SIZE as f64 - rendered_height) / 2.0;

    let target = ImageSurface::create(Format::ARgb32, TAB_FAVICON_SIZE, TAB_FAVICON_SIZE).ok()?;
    let context = Context::new(&target).ok()?;
    context.scale(scale, scale);
    context
        .set_source_surface(&source, offset_x / scale, offset_y / scale)
        .ok()?;
    context.paint().ok()?;
    Some(target)
}

fn set_window_icon(window: &ApplicationWindow) {
    // Keep the themed name for installed builds, but also load the project
    // asset directly so `cargo run` works before the installer updates the
    // user's icon theme.
    window.set_icon_name(Some("org.aegis.Browser"));
    if let Some(icon) = load_app_icon() {
        window.set_icon(Some(&icon));
    }
}

fn load_app_icon() -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    if loader.write(APP_ICON_PNG).is_ok() && loader.close().is_ok() {
        return loader.pixbuf();
    }
    None
}

const AVIF_FALLBACK_SCRIPT: &str = r#"
(function () {
  'use strict';

  // Some Linux WebKitGTK builds do not include an AVIF decoder. WordPress
  // commonly keeps the original raster image beside an AVIF thumbnail. Try
  // same-origin variants only; no third-party image proxy is introduced.
  function fallbackUrls(url) {
    try {
      var parsed = new URL(url, document.baseURI);
      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        return [];
      }
      if (!/\.avif$/i.test(parsed.pathname)) {
        return [];
      }

      var resizedPath = parsed.pathname.replace(/\.avif$/i, '');
      var originalPath = resizedPath.replace(/-\d+x\d+$/i, '');
      var paths = [];
      var seen = {};
      [originalPath, resizedPath].forEach(function (path) {
        ['jpg', 'jpeg', 'png', 'webp'].forEach(function (extension) {
          var candidatePath = path + '.' + extension;
          if (!seen[candidatePath]) {
            seen[candidatePath] = true;
            paths.push(candidatePath);
          }
        });
      });
      return paths.map(function (path) {
        parsed.pathname = path;
        return parsed.toString();
      });
    } catch (_) {
      return [];
    }
  }

  function avifUrls(value) {
    // CSS background declarations place the URL before a closing `)`, while
    // src/srcset values usually end at the extension or a query string.
    if (!value || !/\.avif(?:[?#)\s'\"]|$)/i.test(value)) {
      return [];
    }
    return value.match(
      /https?:\/\/[^\s"')]+\.avif(?:\?[^\s"')]+)?/gi,
    ) || [];
  }

  function probeFallback(node, attribute, value, avifUrl) {
    var candidates = fallbackUrls(avifUrl);
    var candidateIndex = 0;

    function tryNext() {
      if (candidateIndex >= candidates.length) {
        return;
      }
      var probe = document.createElement('img');
      var candidate = candidates[candidateIndex++];
      probe.onload = function () {
        var current = node.getAttribute(attribute);
        if (current && current.indexOf(avifUrl) !== -1) {
          node.setAttribute(attribute, current.replace(avifUrl, candidate));
        }
      };
      probe.onerror = tryNext;
      probe.src = candidate;
    }

    tryNext();
  }

  function rewriteNode(node) {
    if (!(node instanceof Element)) {
      return;
    }
    ['style', 'data-style', 'src', 'srcset', 'data-src', 'data-srcset', 'poster']
      .forEach(function (attribute) {
        var value = node.getAttribute(attribute);
        avifUrls(value).forEach(function (avifUrl) {
          probeFallback(node, attribute, value, avifUrl);
        });
      });
  }

  function rewriteTree(root) {
    rewriteNode(root);
    if (root.querySelectorAll) {
      root.querySelectorAll('[style], [data-style], img, source, video')
        .forEach(rewriteNode);
    }
  }

  rewriteTree(document.documentElement);
  new MutationObserver(function (records) {
    records.forEach(function (record) {
      if (record.type === 'attributes') {
        rewriteNode(record.target);
      } else {
        record.addedNodes.forEach(rewriteTree);
      }
    });
  }).observe(document.documentElement, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ['style', 'data-style', 'src', 'srcset', 'data-src', 'data-srcset', 'poster']
  });
})();
"#;

fn connect_navigation_policy(
    webview: &WebView,
    policy_state: Rc<RefCell<BrowserPolicy>>,
    approved_loads: Rc<RefCell<HashSet<String>>>,
) {
    webview.connect_decide_policy(move |view, decision, decision_type| match decision_type {
        PolicyDecisionType::NewWindowAction => {
            // Gmail and other web apps commonly open message links with
            // target="_blank". Aegis keeps the navigation in the current tab
            // because it does not create unmanaged popup WebViews.
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
            let allowed = WebUrl::parse(&uri)
                .ok()
                .map(|url| {
                    policy_state
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
                approved_loads.borrow_mut().insert(uri.clone());
                let target = view.clone();
                gtk::glib::idle_add_local_once(move || target.load_uri(&uri));
            }
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
            if approved_loads.borrow_mut().remove(&uri) {
                decision.use_();
                return true;
            }

            let allowed = WebUrl::parse(&uri)
                .ok()
                .map(|url| {
                    policy_state
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
        PolicyDecisionType::Response | PolicyDecisionType::__Unknown(_) => false,
        _ => false,
    });
}

#[allow(deprecated)]
fn connect_media_permission_policy(
    webview: &WebView,
    shell: Rc<RefCell<BrowserShell>>,
    tab_id: TabId,
    status: Label,
) {
    webview.connect_permission_request(move |_view, request| {
        if request
            .downcast_ref::<InstallMissingMediaPluginsPermissionRequest>()
            .is_some()
        {
            request.deny();
            if shell.borrow().active_tab_id() == Some(tab_id) {
                status.set_text("Codec de vídeo ausente no sistema");
            }
            return true;
        }

        // Camera, microphone, DRM and other privileged media capabilities do
        // not get an implicit grant. A dedicated permission surface can make
        // these requests explicit later without weakening the default.
        request.deny();
        if shell.borrow().active_tab_id() == Some(tab_id) {
            status.set_text("Permissão de mídia bloqueada por padrão");
        }
        true
    });
}

fn close_tab(
    notebook: &Notebook,
    page_tabs: &Rc<RefCell<Vec<TabId>>>,
    tabs: &Rc<RefCell<HashMap<TabId, TabHandle>>>,
    shell: &Rc<RefCell<BrowserShell>>,
    tab_id: TabId,
    status: &Label,
) {
    if tabs.borrow().len() <= 1 {
        status.set_text("Mantenha pelo menos uma aba aberta");
        return;
    }
    let Some(handle) = tabs.borrow().get(&tab_id).cloned() else {
        status.set_text("Aba inexistente");
        return;
    };
    if shell.borrow_mut().close_tab(tab_id).is_err() {
        status.set_text("Não foi possível fechar a aba");
        return;
    }
    if let Some(page) = notebook.page_num(&handle.webview) {
        page_tabs.borrow_mut().remove(page as usize);
        tabs.borrow_mut().remove(&tab_id);
        notebook.remove_page(Some(page));
    }
}

fn sync_active_tab(
    shell: &Rc<RefCell<BrowserShell>>,
    tab_id: TabId,
    address: &Entry,
    security: &Image,
    status: &Label,
) {
    let shell = shell.borrow();
    let Ok(tab) = shell.tab(tab_id) else {
        return;
    };
    if let Some(url) = tab.current_url() {
        address.set_text(url.as_str());
        set_security_indicator(security, Some(url));
    } else {
        address.set_text("");
        set_security_icon(security, SecurityIcon::Neutral);
        security.set_tooltip_text(Some("Nenhuma conexão ativa"));
    }
    status.set_text(match tab.status() {
        TabStatus::Blank => "Pronto",
        TabStatus::Loading => "Carregando",
        TabStatus::Loaded => "Carregado",
        TabStatus::Failed => "Falha ao carregar",
    });
}

fn set_security_indicator(security: &Image, url: Option<&WebUrl>) {
    match url {
        Some(url) if url.is_secure() => {
            set_security_icon(security, SecurityIcon::Secure);
            security.set_tooltip_text(Some("Conexão segura HTTPS"));
        }
        Some(_) => {
            set_security_icon(security, SecurityIcon::Warning);
            security.set_tooltip_text(Some("Conexão não segura HTTP"));
        }
        None => {
            set_security_icon(security, SecurityIcon::Neutral);
            security.set_tooltip_text(Some("Nenhuma conexão ativa"));
        }
    }
}

fn create_security_icon(icon: SecurityIcon) -> Image {
    let image = Image::new();
    set_security_icon(&image, icon);
    image
}

fn set_security_icon(image: &Image, icon: SecurityIcon) {
    let Ok(surface) = ImageSurface::create(Format::ARgb32, 22, 22) else {
        return;
    };
    let Ok(context) = Context::new(&surface) else {
        return;
    };
    context.set_line_cap(gtk::cairo::LineCap::Round);
    match icon {
        SecurityIcon::Secure => {
            context.set_source_rgb(0.14, 0.53, 0.22);
            context.rectangle(4.0, 9.0, 14.0, 10.0);
            let _ = context.fill();
            context.set_line_width(2.5);
            context.arc(11.0, 9.0, 5.0, std::f64::consts::PI, 0.0);
            let _ = context.stroke();
            context.set_source_rgb(1.0, 1.0, 1.0);
            context.arc(11.0, 14.0, 1.2, 0.0, std::f64::consts::TAU);
            let _ = context.fill();
            context.move_to(11.0, 14.0);
            context.line_to(11.0, 17.0);
            let _ = context.stroke();
        }
        SecurityIcon::Warning => {
            context.set_source_rgb(0.82, 0.58, 0.08);
            context.move_to(11.0, 2.0);
            context.line_to(21.0, 20.0);
            context.line_to(1.0, 20.0);
            context.close_path();
            let _ = context.fill();
            context.set_source_rgb(0.08, 0.08, 0.08);
            context.set_line_width(2.0);
            context.move_to(11.0, 7.0);
            context.line_to(11.0, 14.0);
            let _ = context.stroke();
            context.arc(11.0, 17.0, 1.0, 0.0, std::f64::consts::TAU);
            let _ = context.fill();
        }
        SecurityIcon::Neutral => {
            context.set_source_rgb(0.43, 0.47, 0.51);
            context.arc(11.0, 11.0, 8.0, 0.0, std::f64::consts::TAU);
            let _ = context.fill();
            context.set_source_rgb(1.0, 1.0, 1.0);
            context.arc(11.0, 11.0, 2.0, 0.0, std::f64::consts::TAU);
            let _ = context.fill();
        }
    }
    image.set_from_surface(Some(&surface));
    image.set_size_request(22, 22);
}

fn refresh_bookmarks_bar(
    container: &GtkBox,
    bookmarks: &Rc<RefCell<Vec<(String, String)>>>,
    address: &Entry,
    navigate: &Rc<dyn Fn()>,
) {
    for child in container.children() {
        container.remove(&child);
    }

    if bookmarks.borrow().is_empty() {
        let empty = Label::new(Some("Nenhum favorito salvo"));
        empty.set_xalign(0.0);
        container.pack_start(&empty, false, false, 0);
    } else {
        for (title, url) in bookmarks.borrow().iter().cloned() {
            let button = Button::with_label(&title);
            button.set_tooltip_text(Some(&url));
            let address_for_button = address.clone();
            let navigate_for_button = Rc::clone(navigate);
            button.connect_clicked(move |_| {
                address_for_button.set_text(&url);
                navigate_for_button();
            });
            container.pack_start(&button, false, false, 0);
        }
    }
    container.show_all();
}

fn show_bitwarden_dialog(
    parent: &ApplicationWindow,
    webview: &WebView,
    logins: Vec<super::bitwarden::Login>,
    status: Label,
) {
    let dialog = Dialog::with_buttons(
        Some("Bitwarden — cofre de credenciais"),
        Some(parent),
        DialogFlags::MODAL | DialogFlags::DESTROY_WITH_PARENT,
        &[("Fechar", ResponseType::Close)],
    );
    dialog.set_default_size(420, 320);
    let content = dialog.content_area();
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_vexpand(true);

    let list = GtkBox::new(Orientation::Vertical, 6);
    let scroll = ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.set_vexpand(true);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.add(&list);
    content.pack_start(&scroll, true, true, 0);

    if logins.is_empty() {
        list.pack_start(
            &Label::new(Some("Nenhuma credencial de login encontrada no cofre.")),
            false,
            false,
            0,
        );
    } else {
        for login in logins {
            let label = if login.username.is_empty() {
                login.name.clone()
            } else {
                format!("{} — {}", login.name, login.username)
            };
            let button = Button::with_label(&label);
            button.set_tooltip_text(Some("Preencher usuário e senha"));
            let webview_for_login = webview.clone();
            let status_for_login = status.clone();
            let dialog_for_login = dialog.clone();
            button.connect_clicked(move |_| {
                let script = super::bitwarden::autofill_script(&login);
                webview_for_login.evaluate_javascript(
                    &script,
                    None,
                    None,
                    None::<&gtk::gio::Cancellable>,
                    |_| {},
                );
                status_for_login.set_text("Credencial preenchida pelo Bitwarden");
                dialog_for_login.close();
            });
            list.pack_start(&button, false, false, 0);
        }
    }
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.show_all();
}

fn show_bitwarden_error(parent: &ApplicationWindow, error: &str) {
    let dialog = MessageDialog::new(
        Some(parent),
        DialogFlags::MODAL,
        MessageType::Warning,
        ButtonsType::Close,
        error,
    );
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.show_all();
}

fn sync_navigation_buttons(
    tab_id: TabId,
    tabs: &Rc<RefCell<HashMap<TabId, TabHandle>>>,
    back: &Button,
    forward: &Button,
) {
    if let Some(tab) = tabs.borrow().get(&tab_id) {
        back.set_sensitive(tab.webview.can_go_back());
        forward.set_sensitive(tab.webview.can_go_forward());
    } else {
        back.set_sensitive(false);
        forward.set_sensitive(false);
    }
}

fn tab_display_name(shell: &Rc<RefCell<BrowserShell>>, tab_id: TabId) -> String {
    let shell = shell.borrow();
    let Ok(tab) = shell.tab(tab_id) else {
        return "Nova aba".to_owned();
    };
    let name = tab
        .title()
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| tab.current_url().map(|url| url.origin().host().to_owned()))
        .unwrap_or_else(|| "Nova aba".to_owned());
    name.chars().take(42).collect()
}
