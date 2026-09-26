use aegis_browser_shell::{BrowserShell, NavigationResult, ShellError, TabId, TabStatus};
use aegis_policy_core::BrowserPolicy;
use eframe::egui;

pub struct AegisApp {
    shell: BrowserShell,
    address_bar: String,
    status_message: String,
    pending_confirmation: Option<TabId>,
    settings_open: bool,
}

impl Default for AegisApp {
    fn default() -> Self {
        let mut shell = BrowserShell::new(BrowserPolicy::default());
        shell.new_tab();
        Self {
            shell,
            address_bar: String::new(),
            status_message: "Pronto".to_owned(),
            pending_confirmation: None,
            settings_open: false,
        }
    }
}

impl AegisApp {
    pub fn security_label(&self) -> &'static str {
        match self.shell.active_tab().and_then(|tab| tab.current_url()) {
            Some(url) if url.is_secure() => "HTTPS",
            Some(_) => "HTTP inseguro",
            None => "Nova aba",
        }
    }

    fn select_tab(&mut self, tab_id: TabId) {
        if self.shell.select_tab(tab_id).is_ok() {
            self.sync_address_bar();
            self.status_message.clear();
        }
    }

    fn create_tab(&mut self) {
        self.shell.new_tab();
        self.address_bar.clear();
        self.status_message = "Nova aba".to_owned();
    }

    fn close_tab(&mut self, tab_id: TabId) {
        if let Err(error) = self.shell.close_tab(tab_id) {
            self.status_message = format_error(error);
            return;
        }
        self.sync_address_bar();
    }

    fn navigate(&mut self) {
        let Some(tab_id) = self.shell.active_tab_id() else {
            self.create_tab();
            return;
        };
        let input = self.address_bar.trim().to_owned();
        if input.is_empty() {
            self.status_message = "Digite um endereço".to_owned();
            return;
        }

        match self.shell.navigate(tab_id, &input, true) {
            Ok(NavigationResult::Navigated { .. }) => {
                self.status_message = "Navegação iniciada".to_owned();
            }
            Ok(NavigationResult::ConfirmationRequired { tab_id, .. }) => {
                self.pending_confirmation = Some(tab_id);
                self.status_message = "Confirmação necessária".to_owned();
            }
            Ok(NavigationResult::BlockedInsecure { .. }) => {
                self.status_message = "Navegação HTTP bloqueada".to_owned();
            }
            Err(error) => {
                self.status_message = format_error(error);
            }
        }
    }

    fn sync_address_bar(&mut self) {
        self.address_bar = self
            .shell
            .active_tab()
            .and_then(|tab| tab.current_url())
            .map(|url| url.as_str().to_owned())
            .unwrap_or_default();
    }

    fn draw_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("＋").on_hover_text("Nova aba").clicked() {
                self.create_tab();
            }

            for tab_id in self.shell.tab_ids() {
                let (label, is_active) = match self.shell.tab(tab_id) {
                    Ok(tab) => {
                        let label = tab
                            .title()
                            .filter(|title| !title.is_empty())
                            .map(ToOwned::to_owned)
                            .or_else(|| {
                                tab.current_url().map(|url| url.origin().canonical_string())
                            })
                            .unwrap_or_else(|| format!("Aba {}", tab_id.value() + 1));
                        (label, self.shell.active_tab_id() == Some(tab_id))
                    }
                    Err(_) => continue,
                };

                let response = ui.selectable_label(is_active, label);
                if response.clicked() {
                    self.select_tab(tab_id);
                }
                if ui.small_button("×").on_hover_text("Fechar aba").clicked() {
                    self.close_tab(tab_id);
                    break;
                }
            }
        });
    }

    fn draw_navigation_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.address_bar)
                    .hint_text("Digite uma URL HTTPS")
                    .desired_width((ui.available_width() - 190.0).max(180.0)),
            );
            let enter_pressed =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button("Ir").clicked() || enter_pressed {
                self.navigate();
            }
            if ui.button("Configurações").clicked() {
                self.settings_open = true;
            }
        });
    }

    fn draw_settings(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }

        let initial_policy = self.shell.policy();
        let mut policy = initial_policy;
        let mut close = false;

        egui::SidePanel::right("settings_panel")
            .resizable(false)
            .min_width(340.0)
            .max_width(400.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Configurações");
                    if ui.button("Fechar").clicked() {
                        close = true;
                    }
                });
                ui.label(
                    egui::RichText::new("Proteções aplicadas a novas navegações")
                        .small()
                        .weak(),
                );
                ui.add_space(12.0);

                ui.heading("Segurança");
                ui.add_space(4.0);
                ui.checkbox(&mut policy.https_only, "Exigir HTTPS");
                ui.label(
                    egui::RichText::new(
                        "Bloqueia páginas HTTP comuns e mantém a conexão criptografada como padrão.",
                    )
                    .small()
                    .weak(),
                );

                ui.add_space(10.0);
                ui.add_enabled_ui(!policy.https_only, |ui| {
                    ui.checkbox(
                        &mut policy.confirm_http_exceptions,
                        "Pedir confirmação para exceções HTTP",
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "Quando ativo, uma exceção HTTP precisa de uma confirmação explícita.",
                    )
                    .small()
                    .weak(),
                );

                ui.add_space(10.0);
                ui.checkbox(
                    &mut policy.allow_loopback_http,
                    "Permitir HTTP em localhost",
                );
                ui.label(
                    egui::RichText::new(
                        "Útil para desenvolvimento local; não libera HTTP público.",
                    )
                    .small()
                    .weak(),
                );

                ui.add_space(18.0);
                ui.separator();
                ui.heading("Privacidade");
                ui.add_space(4.0);
                ui.label("Contexto de navegação");
                ui.label(
                    egui::RichText::new("Efêmero — cookies e cache não são preservados entre execuções.")
                        .small()
                        .weak(),
                );
                ui.add_space(8.0);
                ui.label("Downloads");
                ui.label(
                    egui::RichText::new("Bloqueados por padrão até existir uma área de quarentena.")
                        .small()
                        .weak(),
                );

                ui.add_space(18.0);
                ui.separator();
                ui.label(
                    egui::RichText::new("As alterações são aplicadas imediatamente.")
                        .small()
                        .italics()
                        .weak(),
                );
            });

        if policy != initial_policy {
            self.shell.set_policy(policy);
            self.status_message = "Configurações aplicadas".to_owned();
        }
        if close {
            self.settings_open = false;
        }
    }

    fn draw_confirmation(&mut self, ctx: &egui::Context) {
        let Some(tab_id) = self.pending_confirmation else {
            return;
        };

        egui::Window::new("Navegação insegura")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Esta página usa HTTP sem criptografia.");
                ui.label("Deseja continuar mesmo assim?");
                ui.horizontal(|ui| {
                    if ui.button("Cancelar").clicked() {
                        let _ = self.shell.cancel_insecure_navigation(tab_id);
                        self.pending_confirmation = None;
                        self.status_message = "Navegação cancelada".to_owned();
                    }
                    if ui.button("Continuar").clicked() {
                        match self.shell.confirm_insecure_navigation(tab_id) {
                            Ok(_) => {
                                self.pending_confirmation = None;
                                self.status_message = "Navegação HTTP iniciada".to_owned();
                            }
                            Err(error) => {
                                self.pending_confirmation = None;
                                self.status_message = format_error(error);
                            }
                        }
                    }
                });
            });
    }
}

impl eframe::App for AegisApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| self.draw_tabs(ui));
        egui::TopBottomPanel::top("navigation").show(ctx, |ui| {
            self.draw_navigation_bar(ui);
            ui.horizontal(|ui| {
                ui.label(self.security_label());
                ui.separator();
                ui.label(&self.status_message);
            });
        });

        self.draw_settings(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.heading("Aegis Browser");
                ui.label("Shell de navegação com políticas de privacidade e segurança.");
                if let Some(tab) = self.shell.active_tab() {
                    if tab.status() == TabStatus::Failed {
                        ui.colored_label(egui::Color32::RED, "Falha ao carregar a página");
                    }
                }
            });
        });

        self.draw_confirmation(ctx);
    }
}

fn format_error(error: ShellError) -> String {
    match error {
        ShellError::UnknownTab(_) => "Aba inexistente".to_owned(),
        ShellError::NoPendingNavigation(_) => "Nenhuma confirmação pendente".to_owned(),
        ShellError::InvalidUrl(_) => "URL inválida ou esquema não permitido".to_owned(),
        ShellError::Permission(_) => "Permissão recusada".to_owned(),
        ShellError::Engine(_) => "A engine recusou a navegação".to_owned(),
        ShellError::InvalidEngineEvent(_) => "Evento inválido da engine".to_owned(),
    }
}
