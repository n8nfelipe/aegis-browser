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
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, ButtonsType, Dialog, DialogFlags, Entry,
    EntryCompletion, IconSize, Image, Label, ListStore, MessageDialog, MessageType, Notebook,
    Orientation, PackType, PositionType, ProgressBar, ResponseType, ScrolledWindow,
};
#[allow(deprecated)]
use webkit2gtk::InstallMissingMediaPluginsPermissionRequest;
use webkit2gtk::{
    HardwareAccelerationPolicy, LoadEvent, NavigationPolicyDecision, NavigationPolicyDecisionExt,
    PermissionRequestExt, PolicyDecisionExt, PolicyDecisionType, ResponsePolicyDecision,
    ResponsePolicyDecisionExt, Settings, SettingsExt, URIRequestExt, URIResponseExt,
    UserContentInjectedFrames, UserContentManager, UserContentManagerExt, UserScript,
    UserScriptInjectionTime, WebContext, WebInspectorExt, WebView, WebViewExt,
};

use super::{
    configure_download_policy, load_download_history, new_profile_context,
    request_external_download, setup_favicon_cache_cleanup, shell_error_text,
    show_downloads_dialog, show_http_confirmation, show_settings_dialog, DownloadHistory,
    GtkEngine, ProfileContexts,
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
    let loaded_preferences = super::load_preferences();
    let policy_state = Rc::new(RefCell::new(BrowserPolicy {
        https_only: loaded_preferences.https_only,
        allow_loopback_http: loaded_preferences.allow_loopback_http,
        confirm_http_exceptions: loaded_preferences.confirm_http_exceptions,
    }));
    let shell = Rc::new(RefCell::new(BrowserShell::new(*policy_state.borrow())));
    let preferences = Rc::new(RefCell::new(loaded_preferences));
    super::apply_theme(preferences.borrow().theme);
    let preferences_for_shutdown = Rc::clone(&preferences);
    application.connect_shutdown(move |_| {
        let preferences_snapshot = preferences_for_shutdown.borrow().clone();
        let _ = super::save_preferences(&preferences_snapshot);
    });
    let favicon_cache_dirs = setup_favicon_cache_cleanup(application);
    let mut initial_profile_contexts = HashMap::new();
    for profile in &preferences.borrow().profiles {
        initial_profile_contexts.insert(
            profile.clone(),
            new_profile_context(profile, &favicon_cache_dirs),
        );
    }
    let context = initial_profile_contexts
        .get("Pessoal")
        .cloned()
        .unwrap_or_else(|| new_profile_context("Pessoal", &favicon_cache_dirs));
    let profile_contexts: ProfileContexts = Rc::new(RefCell::new(initial_profile_contexts));
    profile_contexts
        .borrow_mut()
        .entry("Pessoal".to_owned())
        .or_insert_with(|| context.clone());
    let download_history: DownloadHistory = load_download_history();
    let tabs: Rc<RefCell<HashMap<TabId, TabHandle>>> = Rc::new(RefCell::new(HashMap::new()));
    let page_tabs = Rc::new(RefCell::new(Vec::<TabId>::new()));

    let window = ApplicationWindow::new(application);
    window.set_title("Aegis Browser — WebKitGTK");
    set_window_icon(&window);
    window.set_default_size(1200, 800);
    let preferences_for_window_close = Rc::clone(&preferences);
    window.connect_destroy(move |_| {
        let preferences_snapshot = preferences_for_window_close.borrow().clone();
        let _ = super::save_preferences(&preferences_snapshot);
    });

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
    let new_tab = Button::with_label("+");
    new_tab.set_size_request(30, -1);
    new_tab.set_focus_on_click(false);
    new_tab.set_tooltip_text(Some("Abrir nova aba (Ctrl+Shift+T)"));
    let settings = Button::from_icon_name(Some("preferences-system"), IconSize::Button);
    settings.set_tooltip_text(Some("Configurações"));
    let add_bookmark = Button::with_label("☆");
    add_bookmark.set_tooltip_text(Some("Adicionar página aos favoritos"));
    let bitwarden_button = Button::from_icon_name(Some("dialog-password"), IconSize::Button);
    bitwarden_button.set_tooltip_text(Some("Preencher credencial com Bitwarden"));
    let downloads_button = Button::from_icon_name(Some("folder-download"), IconSize::Button);
    downloads_button.set_tooltip_text(Some("Mostrar arquivos baixados"));
    let extension_toolbar = GtkBox::new(Orientation::Horizontal, 2);
    let bookmarks_bar = GtkBox::new(Orientation::Horizontal, 6);
    bookmarks_bar.set_margin_start(12);
    bookmarks_bar.set_margin_end(12);
    bookmarks_bar.set_margin_bottom(6);
    bookmarks_bar.set_visible(preferences.borrow().show_bookmarks_bar);
    let bookmarks: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(load_bookmarks()));
    let session_suggestions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let autocomplete_model = ListStore::new(&[String::static_type(), String::static_type()]);
    let autocomplete = EntryCompletion::new();
    autocomplete.set_model(Some(&autocomplete_model));
    autocomplete.set_text_column(0);
    autocomplete.set_inline_completion(false);
    autocomplete.set_popup_completion(true);
    autocomplete.set_popup_set_width(true);
    autocomplete.set_minimum_key_length(1);
    address.set_completion(Some(&autocomplete));
    refresh_address_completion(&autocomplete_model, &bookmarks, &session_suggestions);
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
    toolbar.pack_start(&extension_toolbar, false, false, 0);
    toolbar.pack_start(&settings, false, false, 0);
    toolbar.pack_start(&downloads_button, false, false, 0);
    toolbar.pack_start(&status, false, false, 0);
    root.pack_start(&toolbar, false, false, 0);
    root.pack_start(&progress, false, false, 0);
    root.pack_start(&bookmarks_bar, false, false, 0);

    let notebook = Notebook::new();
    notebook.set_show_tabs(true);
    notebook.set_tab_pos(PositionType::Top);
    notebook.set_scrollable(true);
    notebook.set_action_widget(&new_tab, PackType::Start);
    new_tab.show();
    root.pack_start(&notebook, true, true, 0);
    window.add(&root);
    super::refresh_extensions_toolbar(&extension_toolbar);

    for profile_context in profile_contexts.borrow().values() {
        configure_download_policy(
            profile_context,
            &window,
            status.clone(),
            Some(downloads_button.clone()),
            Rc::clone(&download_history),
        );
    }

    let window_for_downloads_button = window.clone();
    let download_history_for_button = Rc::clone(&download_history);
    downloads_button.connect_clicked(move |_| {
        show_downloads_dialog(
            &window_for_downloads_button,
            Rc::clone(&download_history_for_button),
        );
    });

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
    let window_for_new_tab = window.clone();
    let downloads_button_for_new_tab = downloads_button.clone();
    let download_history_for_new_tab = Rc::clone(&download_history);
    let create_new_tab: Rc<dyn Fn()> = Rc::new(move || {
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
            &window_for_new_tab,
            downloads_button_for_new_tab.clone(),
            Rc::clone(&download_history_for_new_tab),
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
    let create_new_tab_for_button = Rc::clone(&create_new_tab);
    new_tab.connect_clicked(move |_| create_new_tab_for_button());

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
        &window,
        downloads_button.clone(),
        Rc::clone(&download_history),
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
    let create_new_tab_for_keys = Rc::clone(&create_new_tab);
    window.connect_key_press_event(move |_window, event| {
        let key = event.keyval();
        let ctrl_shift_new_tab = event.state().contains(gtk::gdk::ModifierType::CONTROL_MASK)
            && event.state().contains(gtk::gdk::ModifierType::SHIFT_MASK)
            && (key == gtk::gdk::keys::constants::T || key == gtk::gdk::keys::constants::t);
        let ctrl_reload = event.state().contains(gtk::gdk::ModifierType::CONTROL_MASK)
            && (key == gtk::gdk::keys::constants::R || key == gtk::gdk::keys::constants::r);
        if ctrl_shift_new_tab {
            create_new_tab_for_keys();
            gtk::glib::Propagation::Stop
        } else if ctrl_reload || key == gtk::gdk::keys::constants::F5 {
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

    let notebook_for_devtools = notebook.clone();
    let page_tabs_for_devtools = Rc::clone(&page_tabs);
    let tabs_for_devtools = Rc::clone(&tabs);
    let status_for_devtools = status.clone();
    let show_devtools: Rc<dyn Fn()> = Rc::new(move || {
        let Some(page) = notebook_for_devtools.current_page() else {
            status_for_devtools.set_text("Nenhuma aba ativa");
            return;
        };
        let Some(tab_id) = page_tabs_for_devtools.borrow().get(page as usize).copied() else {
            status_for_devtools.set_text("Aba inválida");
            return;
        };
        let Some(tab) = tabs_for_devtools.borrow().get(&tab_id).cloned() else {
            status_for_devtools.set_text("Aba inexistente");
            return;
        };
        if let Some(inspector) = tab.webview.inspector() {
            inspector.show();
            status_for_devtools.set_text("DevTools abertas");
        } else {
            status_for_devtools.set_text("DevTools indisponíveis nesta aba");
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
    let downloads_button_for_settings = downloads_button.clone();
    let download_history_for_settings = Rc::clone(&download_history);
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
            Some(Rc::clone(&show_devtools)),
            Some(downloads_button_for_settings.clone()),
            Rc::clone(&download_history_for_settings),
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
        let autocomplete_model = autocomplete_model.clone();
        let bookmarks = Rc::clone(&bookmarks);
        let session_suggestions = Rc::clone(&session_suggestions);
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
                    remember_address_input(
                        &session_suggestions,
                        &autocomplete_model,
                        &bookmarks,
                        &input,
                    );
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
                    remember_address_input(
                        &session_suggestions,
                        &autocomplete_model,
                        &bookmarks,
                        &input,
                    );
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
    let address_for_completion = address.clone();
    let navigate_for_completion = Rc::clone(&navigate);
    autocomplete.connect_match_selected(move |_, model, iter| {
        if let Ok(url) = model.value(iter, 1).get::<String>() {
            address_for_completion.set_text(&url);
            address_for_completion.set_position(-1);
            navigate_for_completion();
        }
        glib::Propagation::Stop
    });
    let navigate_for_address = Rc::clone(&navigate);
    address.connect_activate(move |_| navigate_for_address());

    let bookmarks_for_add = Rc::clone(&bookmarks);
    let bookmarks_bar_for_add = bookmarks_bar.clone();
    let address_for_bookmark = address.clone();
    let notebook_for_bookmark = notebook.clone();
    let page_tabs_for_bookmark = Rc::clone(&page_tabs);
    let tabs_for_bookmark = Rc::clone(&tabs);
    let status_for_bookmark = status.clone();
    let navigate_for_bookmark = Rc::clone(&navigate);
    let autocomplete_model_for_bookmark = autocomplete_model.clone();
    let session_suggestions_for_bookmark = Rc::clone(&session_suggestions);
    add_bookmark.connect_clicked(move |_| {
        let current_tab_url = notebook_for_bookmark
            .current_page()
            .and_then(|page| page_tabs_for_bookmark.borrow().get(page as usize).copied())
            .and_then(|tab_id| tabs_for_bookmark.borrow().get(&tab_id).cloned())
            .and_then(|tab| tab.webview.uri().map(|uri| uri.to_string()))
            .filter(|url| url.starts_with("http://") || url.starts_with("https://"));
        let url = current_tab_url.unwrap_or_else(|| address_for_bookmark.text().trim().to_owned());
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            status_for_bookmark.set_text("Abra uma página HTTP ou HTTPS para salvar");
            return;
        }
        let mut bookmarks = bookmarks_for_add.borrow_mut();
        if bookmarks.iter().any(|(_, saved_url)| saved_url == &url) {
            status_for_bookmark.set_text("Página já está nos favoritos");
            return;
        }
        bookmarks.push((url.clone(), url));
        let save_result = save_bookmarks(&bookmarks);
        drop(bookmarks);
        refresh_bookmarks_bar(
            &bookmarks_bar_for_add,
            &bookmarks_for_add,
            &address_for_bookmark,
            &navigate_for_bookmark,
            &status_for_bookmark,
        );
        refresh_address_completion(
            &autocomplete_model_for_bookmark,
            &bookmarks_for_add,
            &session_suggestions_for_bookmark,
        );
        match save_result {
            Ok(()) => status_for_bookmark.set_text("Favorito salvo"),
            Err(error) => status_for_bookmark
                .set_text(&format!("Favorito salvo apenas nesta sessão: {error}")),
        }
    });
    refresh_bookmarks_bar(&bookmarks_bar, &bookmarks, &address, &navigate, &status);

    window.show_all();
}

fn refresh_address_completion(
    model: &ListStore,
    bookmarks: &Rc<RefCell<Vec<(String, String)>>>,
    session_suggestions: &Rc<RefCell<Vec<String>>>,
) {
    model.clear();
    let mut seen = HashSet::new();
    let mut suggestions = Vec::new();

    for value in session_suggestions.borrow().iter().rev() {
        if seen.insert(value.clone()) {
            suggestions.push((value.clone(), value.clone()));
        }
    }
    for (title, url) in bookmarks.borrow().iter() {
        if seen.insert(url.clone()) {
            let title = title.trim();
            let display = if title.is_empty() || title == url {
                url.clone()
            } else {
                format!("{title}  ·  {url}")
            };
            suggestions.push((display, url.clone()));
        }
    }

    for (display, value) in suggestions.into_iter().take(24) {
        model.insert_with_values(None, &[(0, &display), (1, &value)]);
    }
}

fn remember_address_input(
    session_suggestions: &Rc<RefCell<Vec<String>>>,
    model: &ListStore,
    bookmarks: &Rc<RefCell<Vec<(String, String)>>>,
    input: &str,
) {
    let input = input.trim();
    if input.is_empty() {
        return;
    }

    let mut suggestions = session_suggestions.borrow_mut();
    suggestions.retain(|value| value != input);
    suggestions.push(input.to_owned());
    if suggestions.len() > 24 {
        suggestions.remove(0);
    }
    drop(suggestions);
    refresh_address_completion(model, bookmarks, session_suggestions);
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
    window: &ApplicationWindow,
    downloads_button: Button,
    download_history: DownloadHistory,
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
    settings.set_enable_developer_extras(true);
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
    connect_navigation_policy(
        &webview,
        policy_state,
        Rc::clone(&approved_loads),
        window,
        status.clone(),
        downloads_button,
        download_history,
    );
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
    window.set_icon_name(Some("io.github.n8nfelipe.aegis-browser"));
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
    window: &ApplicationWindow,
    status: Label,
    downloads_button: Button,
    download_history: DownloadHistory,
) {
    let window = window.clone();
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
        PolicyDecisionType::Response => {
            let Some(response) = decision.downcast_ref::<ResponsePolicyDecision>() else {
                return false;
            };
            // A clicked download link can be reported without the main-frame
            // flag (for example when the site uses target=_blank). Route any
            // unsupported response through the explicit download flow.
            if !response.is_mime_type_supported() {
                let Some(source_url) = response.request().and_then(|request| request.uri()) else {
                    decision.ignore();
                    status.set_text("Download recusado: URL indisponível");
                    return true;
                };
                let source_url = source_url.to_string();
                let suggested_filename = response
                    .response()
                    .and_then(|response| response.suggested_filename())
                    .map(|filename| super::safe_download_filename(filename.as_str()))
                    .unwrap_or_else(|| super::suggested_filename_from_url(&source_url));
                decision.ignore();
                request_external_download(
                    &window,
                    status.clone(),
                    Some(downloads_button.clone()),
                    Rc::clone(&download_history),
                    source_url,
                    suggested_filename,
                );
                return true;
            }
            false
        }
        PolicyDecisionType::__Unknown(_) => false,
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
        // WebKitGTK 2.42+ uses a dedicated permission request for the
        // asynchronous Clipboard API (`navigator.clipboard`). The Rust
        // bindings used here predate that wrapper, so identify the request by
        // its registered GObject type while keeping every other permission
        // denied by default below.
        if request.type_().name() == "WebKitClipboardPermissionRequest" {
            request.allow();
            return true;
        }

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

fn bookmarks_path() -> std::path::PathBuf {
    super::user_data_root()
        .join("aegis-browser")
        .join("bookmarks.json")
}

fn load_bookmarks() -> Vec<(String, String)> {
    let path = bookmarks_path();
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(bookmarks) = serde_json::from_str::<Vec<(String, String)>>(&contents) else {
        return Vec::new();
    };
    bookmarks
        .into_iter()
        .filter(|(_, url)| url.starts_with("http://") || url.starts_with("https://"))
        .collect()
}

fn save_bookmarks(bookmarks: &[(String, String)]) -> Result<(), String> {
    let path = bookmarks_path();
    let parent = path
        .parent()
        .ok_or_else(|| "diretório dos favoritos inválido".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("não foi possível criar a pasta dos favoritos: {error}"))?;
    let contents = serde_json::to_vec_pretty(bookmarks)
        .map_err(|error| format!("não foi possível serializar os favoritos: {error}"))?;
    let temporary_path = path.with_extension("json.tmp");
    std::fs::write(&temporary_path, contents)
        .map_err(|error| format!("não foi possível gravar os favoritos: {error}"))?;
    std::fs::rename(&temporary_path, &path)
        .map_err(|error| format!("não foi possível finalizar os favoritos: {error}"))
}

fn refresh_bookmarks_bar(
    container: &GtkBox,
    bookmarks: &Rc<RefCell<Vec<(String, String)>>>,
    address: &Entry,
    navigate: &Rc<dyn Fn()>,
    status: &Label,
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
            let item = GtkBox::new(Orientation::Horizontal, 0);
            let button = Button::with_label(&title);
            button.set_tooltip_text(Some(&url));
            let address_for_button = address.clone();
            let navigate_for_button = Rc::clone(navigate);
            let url_for_button = url.clone();
            button.connect_clicked(move |_| {
                address_for_button.set_text(&url_for_button);
                navigate_for_button();
            });

            let remove = Button::from_icon_name(Some("edit-delete"), IconSize::Menu);
            remove.set_tooltip_text(Some("Excluir favorito"));
            let bookmarks_for_remove = Rc::clone(bookmarks);
            let container_for_remove = container.clone();
            let address_for_remove = address.clone();
            let navigate_for_remove = Rc::clone(navigate);
            let status_for_remove = status.clone();
            let url_for_remove = url.clone();
            remove.connect_clicked(move |_| {
                let removed = {
                    let mut bookmarks = bookmarks_for_remove.borrow_mut();
                    let Some(index) = bookmarks
                        .iter()
                        .position(|(_, saved_url)| saved_url == &url_for_remove)
                    else {
                        return;
                    };
                    let bookmark = bookmarks.remove(index);
                    let save_result = save_bookmarks(&bookmarks);
                    if save_result.is_err() {
                        bookmarks.insert(index, bookmark);
                    }
                    save_result
                };

                refresh_bookmarks_bar(
                    &container_for_remove,
                    &bookmarks_for_remove,
                    &address_for_remove,
                    &navigate_for_remove,
                    &status_for_remove,
                );
                match removed {
                    Ok(()) => status_for_remove.set_text("Favorito excluído"),
                    Err(error) => status_for_remove
                        .set_text(&format!("Não foi possível excluir o favorito: {error}")),
                }
            });

            item.pack_start(&button, true, true, 0);
            item.pack_start(&remove, false, false, 0);
            container.pack_start(&item, false, false, 0);
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
