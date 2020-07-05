use crate::config::{Config, RootsConfig};
use crate::lang::Translator;
use crate::manifest::{Manifest, SteamMetadata, Store};
use crate::prelude::{
    app_dir, back_up_game, game_file_restoration_target, prepare_backup_target, restore_game, scan_game_for_backup,
    scan_game_for_restoration, Error, ScanInfo,
};

use iced::{
    button, executor, scrollable, text_input, Align, Application, Button, Column, Command, Container, Element,
    HorizontalAlignment, Length, Radio, Row, Scrollable, Space, Text, TextInput,
};

#[derive(Default)]
struct App {
    config: Config,
    manifest: Manifest,
    translator: Translator,
    operation: Option<OngoingOperation>,
    screen: Screen,
    modal_theme: Option<ModalTheme>,
    original_working_dir: std::path::PathBuf,
    modal: ModalComponent,
    backup_screen: BackupScreenComponent,
    restore_screen: RestoreScreenComponent,
}

#[derive(Debug, Clone)]
enum Message {
    Idle,
    ConfirmBackupStart,
    BackupStart,
    ConfirmRestoreStart,
    RestoreStart,
    PreviewBackupStart,
    PreviewRestoreStart,
    BackupStep { game: String, info: ScanInfo },
    RestoreStep { game: String, info: ScanInfo },
    EditedBackupTarget(String),
    EditedRestoreSource(String),
    EditedRootPath(usize, String),
    EditedRootStore(usize, Store),
    AddRoot,
    RemoveRoot(usize),
    SwitchScreenToRestore,
    SwitchScreenToBackup,
}

#[derive(Debug, Clone, PartialEq)]
enum OngoingOperation {
    Backup,
    PreviewBackup,
    Restore,
    PreviewRestore,
}

#[derive(Debug, Clone, PartialEq)]
enum Screen {
    Backup,
    Restore,
}

#[derive(Debug, Clone, PartialEq)]
enum ModalTheme {
    Error { variant: Error },
    ConfirmBackup,
    ConfirmRestore,
}

impl Default for Screen {
    fn default() -> Self {
        Self::Backup
    }
}

#[derive(Default)]
struct ModalComponent {
    positive_button: button::State,
    negative_button: button::State,
}

impl ModalComponent {
    fn view(&mut self, theme: &ModalTheme, translator: &Translator, config: &Config) -> Container<Message> {
        let positive_button = Button::new(
            &mut self.positive_button,
            Text::new(match theme {
                ModalTheme::Error { .. } => translator.okay_button(),
                _ => translator.continue_button(),
            })
            .horizontal_alignment(HorizontalAlignment::Center),
        )
        .on_press(match theme {
            ModalTheme::Error { .. } => Message::Idle,
            ModalTheme::ConfirmBackup => Message::BackupStart,
            ModalTheme::ConfirmRestore => Message::RestoreStart,
        })
        .width(Length::Units(125))
        .style(style::Button::Primary);

        let negative_button = Button::new(
            &mut self.negative_button,
            Text::new(translator.cancel_button()).horizontal_alignment(HorizontalAlignment::Center),
        )
        .on_press(Message::Idle)
        .width(Length::Units(125))
        .style(style::Button::Negative);

        Container::new(
            Column::new()
                .padding(5)
                .align_items(Align::Center)
                .push(match theme {
                    ModalTheme::Error { .. } => Row::new()
                        .padding(20)
                        .spacing(20)
                        .align_items(Align::Center)
                        .push(positive_button),
                    _ => Row::new()
                        .padding(20)
                        .spacing(20)
                        .align_items(Align::Center)
                        .push(positive_button)
                        .push(negative_button),
                })
                .push(
                    Row::new()
                        .padding(20)
                        .spacing(20)
                        .align_items(Align::Center)
                        .push(Text::new(match theme {
                            ModalTheme::Error { variant } => translator.handle_error(variant),
                            ModalTheme::ConfirmBackup => translator.modal_confirm_backup(
                                &crate::path::absolute(&config.backup.path),
                                crate::path::exists(&config.backup.path),
                            ),
                            ModalTheme::ConfirmRestore => {
                                translator.modal_confirm_restore(&crate::path::absolute(&config.restore.path))
                            }
                        }))
                        .height(Length::Fill),
                ),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .center_x()
    }
}

struct GameListEntry {
    name: String,
    files: std::collections::HashSet<String>,
    registry_keys: std::collections::HashSet<String>,
}

impl GameListEntry {
    fn view(&mut self, restoring: bool) -> Container<Message> {
        let mut lines = Vec::<String>::new();

        for item in itertools::sorted(&self.files) {
            if restoring {
                if let Ok(target) = game_file_restoration_target(&item) {
                    lines.push(target);
                }
            } else {
                lines.push(item.clone());
            }
        }
        for item in itertools::sorted(&self.registry_keys) {
            lines.push(item.clone());
        }

        Container::new(
            Column::new()
                .padding(5)
                .spacing(5)
                .align_items(Align::Center)
                .push(
                    Row::new().push(
                        Container::new(Text::new(self.name.clone()))
                            .align_x(Align::Center)
                            .width(Length::Fill)
                            .padding(2)
                            .style(style::Container::GameListEntryTitle),
                    ),
                )
                .push(
                    Row::new().push(
                        Container::new(Text::new(lines.join("\n")))
                            .width(Length::Fill)
                            .style(style::Container::GameListEntryBody),
                    ),
                ),
        )
        .style(style::Container::GameListEntry)
    }
}

#[derive(Default)]
struct GameList {
    entries: Vec<GameListEntry>,
    scroll: scrollable::State,
}

impl GameList {
    fn view(&mut self, restoring: bool) -> Container<Message> {
        self.entries.sort_by_key(|x| x.name.clone());
        Container::new({
            self.entries.iter_mut().enumerate().fold(
                Scrollable::new(&mut self.scroll).width(Length::Fill).padding(10),
                |parent: Scrollable<'_, Message>, (_i, x)| {
                    parent
                        .push(x.view(restoring))
                        .push(Space::new(Length::Units(0), Length::Units(10)))
                },
            )
        })
    }
}

#[derive(Default)]
struct RootEditor {
    scroll: scrollable::State,
    rows: Vec<(button::State, text_input::State)>,
}

impl RootEditor {
    fn view(&mut self, config: &Config, translator: &Translator) -> Container<Message> {
        let roots = config.roots.clone();
        if roots.is_empty() {
            Container::new(Text::new(translator.no_roots_are_configured()))
        } else {
            Container::new({
                self.rows.iter_mut().enumerate().fold(
                    Scrollable::new(&mut self.scroll).width(Length::Fill).max_height(100),
                    |parent: Scrollable<'_, Message>, (i, x)| {
                        parent
                            .push(
                                Row::new()
                                    .push(
                                        Button::new(
                                            &mut x.0,
                                            Text::new(translator.remove_root_button())
                                                .horizontal_alignment(HorizontalAlignment::Center)
                                                .size(14),
                                        )
                                        .on_press(Message::RemoveRoot(i))
                                        .style(style::Button::Negative),
                                    )
                                    .push(Space::new(Length::Units(20), Length::Units(0)))
                                    .push(
                                        TextInput::new(&mut x.1, "", &roots[i].path, move |v| {
                                            Message::EditedRootPath(i, v)
                                        })
                                        .width(Length::FillPortion(3))
                                        .padding(5),
                                    )
                                    .push(Space::new(Length::Units(20), Length::Units(0)))
                                    .push({
                                        Radio::new(
                                            Store::Steam,
                                            translator.store(&Store::Steam),
                                            Some(roots[i].store),
                                            move |v| Message::EditedRootStore(i, v),
                                        )
                                    })
                                    .push({
                                        Radio::new(
                                            Store::Other,
                                            translator.store(&Store::Other),
                                            Some(roots[i].store),
                                            move |v| Message::EditedRootStore(i, v),
                                        )
                                    }),
                            )
                            .push(Row::new().push(Space::new(Length::Units(0), Length::Units(5))))
                    },
                )
            })
        }
    }
}

#[derive(Default)]
struct BackupScreenComponent {
    total_games: usize,
    log: GameList,
    start_button: button::State,
    preview_button: button::State,
    nav_button: button::State,
    add_root_button: button::State,
    backup_target_input: text_input::State,
    root_editor: RootEditor,
}

impl BackupScreenComponent {
    fn new(config: &Config) -> Self {
        let mut root_editor = RootEditor::default();
        while root_editor.rows.len() < config.roots.len() {
            root_editor
                .rows
                .push((button::State::default(), text_input::State::default()));
        }

        Self {
            root_editor,
            ..Default::default()
        }
    }

    fn view(&mut self, config: &Config, translator: &Translator, allow_input: bool) -> Container<Message> {
        Container::new(
            Column::new()
                .padding(5)
                .align_items(Align::Center)
                .push(
                    Row::new()
                        .padding(20)
                        .spacing(20)
                        .align_items(Align::Center)
                        .push(
                            Button::new(
                                &mut self.preview_button,
                                Text::new(translator.preview_button())
                                    .horizontal_alignment(HorizontalAlignment::Center),
                            )
                            .on_press(Message::PreviewBackupStart)
                            .width(Length::Units(125))
                            .style(if !allow_input {
                                style::Button::Disabled
                            } else {
                                style::Button::Primary
                            }),
                        )
                        .push(
                            Button::new(
                                &mut self.start_button,
                                Text::new(translator.backup_button()).horizontal_alignment(HorizontalAlignment::Center),
                            )
                            .on_press(Message::ConfirmBackupStart)
                            .width(Length::Units(125))
                            .style(if !allow_input {
                                style::Button::Disabled
                            } else {
                                style::Button::Primary
                            }),
                        )
                        .push(
                            Button::new(
                                &mut self.add_root_button,
                                Text::new(translator.add_root_button())
                                    .horizontal_alignment(HorizontalAlignment::Center),
                            )
                            .on_press(Message::AddRoot)
                            .width(Length::Units(125))
                            .style(style::Button::Primary),
                        )
                        .push(
                            Button::new(
                                &mut self.nav_button,
                                Text::new(translator.nav_restore_button())
                                    .horizontal_alignment(HorizontalAlignment::Center),
                            )
                            .on_press(Message::SwitchScreenToRestore)
                            .width(Length::Units(125))
                            .style(style::Button::Navigation),
                        ),
                )
                .push(
                    Row::new()
                        .padding(20)
                        .align_items(Align::Center)
                        .push(Text::new(translator.processed_games(self.total_games)).size(50)),
                )
                .push(
                    Row::new()
                        .padding(20)
                        .align_items(Align::Center)
                        .push(Text::new(translator.backup_target_label()))
                        .push(Space::new(Length::Units(20), Length::Units(0)))
                        .push(
                            TextInput::new(
                                &mut self.backup_target_input,
                                "",
                                &config.backup.path,
                                Message::EditedBackupTarget,
                            )
                            .padding(5),
                        ),
                )
                .push(self.root_editor.view(&config, &translator))
                .push(Space::new(Length::Units(0), Length::Units(30)))
                .push(self.log.view(false)),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .center_x()
    }
}

#[derive(Default)]
struct RestoreScreenComponent {
    total_games: usize,
    log: GameList,
    start_button: button::State,
    preview_button: button::State,
    nav_button: button::State,
    restore_source_input: text_input::State,
}

impl RestoreScreenComponent {
    fn view(&mut self, config: &Config, translator: &Translator, allow_input: bool) -> Container<Message> {
        Container::new(
            Column::new()
                .padding(5)
                .align_items(Align::Center)
                .push(
                    Row::new()
                        .padding(20)
                        .spacing(20)
                        .align_items(Align::Center)
                        .push(
                            Button::new(
                                &mut self.preview_button,
                                Text::new(translator.preview_button())
                                    .horizontal_alignment(HorizontalAlignment::Center),
                            )
                            .on_press(Message::PreviewRestoreStart)
                            .width(Length::Units(125))
                            .style(if !allow_input {
                                style::Button::Disabled
                            } else {
                                style::Button::Primary
                            }),
                        )
                        .push(
                            Button::new(
                                &mut self.start_button,
                                Text::new(translator.restore_button())
                                    .horizontal_alignment(HorizontalAlignment::Center),
                            )
                            .on_press(Message::ConfirmRestoreStart)
                            .width(Length::Units(125))
                            .style(if !allow_input {
                                style::Button::Disabled
                            } else {
                                style::Button::Primary
                            }),
                        )
                        .push(
                            Button::new(
                                &mut self.nav_button,
                                Text::new(translator.nav_backup_button())
                                    .horizontal_alignment(HorizontalAlignment::Center),
                            )
                            .on_press(Message::SwitchScreenToBackup)
                            .width(Length::Units(125))
                            .style(style::Button::Navigation),
                        ),
                )
                .push(
                    Row::new()
                        .padding(20)
                        .align_items(Align::Center)
                        .push(Text::new(translator.processed_games(self.total_games)).size(50)),
                )
                .push(
                    Row::new()
                        .padding(20)
                        .align_items(Align::Center)
                        .push(Text::new(translator.restore_source_label()))
                        .push(Space::new(Length::Units(20), Length::Units(0)))
                        .push(
                            TextInput::new(
                                &mut self.restore_source_input,
                                "",
                                &config.restore.path,
                                Message::EditedRestoreSource,
                            )
                            .padding(5),
                        ),
                )
                .push(Space::new(Length::Units(0), Length::Units(30)))
                .push(self.log.view(true)),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .center_x()
    }
}

impl Application for App {
    type Executor = executor::Default;
    type Message = Message;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Message>) {
        let translator = Translator::default();
        let mut modal_theme: Option<ModalTheme> = None;
        let mut config = match Config::load() {
            Ok(x) => x,
            Err(x) => {
                modal_theme = Some(ModalTheme::Error { variant: x });
                Config::default()
            }
        };
        let manifest = match Manifest::load(&mut config) {
            Ok(x) => x,
            Err(x) => {
                modal_theme = Some(ModalTheme::Error { variant: x });
                Manifest::default()
            }
        };

        (
            Self {
                backup_screen: BackupScreenComponent::new(&config),
                translator,
                config,
                manifest,
                original_working_dir: std::env::current_dir().unwrap(),
                modal_theme,
                ..Self::default()
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        self.translator.window_title()
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Idle => {
                self.operation = None;
                self.modal_theme = None;
                std::env::set_current_dir(&self.original_working_dir).unwrap();
                Command::none()
            }
            Message::ConfirmBackupStart => {
                self.modal_theme = Some(ModalTheme::ConfirmBackup);
                Command::none()
            }
            Message::ConfirmRestoreStart => {
                self.modal_theme = Some(ModalTheme::ConfirmRestore);
                Command::none()
            }
            Message::BackupStart => {
                if self.operation.is_some() {
                    return Command::none();
                }

                self.backup_screen.total_games = 0;
                self.backup_screen.log.entries.clear();
                self.modal_theme = None;

                let backup_path = crate::path::absolute(&self.config.backup.path);
                if let Err(e) = prepare_backup_target(&backup_path) {
                    self.modal_theme = Some(ModalTheme::Error { variant: e });
                    return Command::none();
                }

                self.config.save();
                self.operation = Some(OngoingOperation::Backup);

                std::env::set_current_dir(app_dir()).unwrap();

                let mut commands: Vec<Command<Message>> = vec![];
                for key in self.manifest.0.iter().map(|(k, _)| k.clone()) {
                    let game = self.manifest.0[&key].clone();
                    let roots = self.config.roots.clone();
                    let key2 = key.clone();
                    let backup_path2 = backup_path.clone();
                    let steam_id = game.steam.clone().unwrap_or(SteamMetadata { id: None }).id;
                    commands.push(Command::perform(
                        async move {
                            let info =
                                scan_game_for_backup(&game, &key, &roots, &app_dir().to_string_lossy(), &steam_id);
                            back_up_game(&info, &backup_path2, &key);
                            info
                        },
                        move |info| Message::BackupStep {
                            game: key2.clone(),
                            info,
                        },
                    ));
                }

                commands.push(Command::perform(async move {}, move |_| Message::Idle));
                Command::batch(commands)
            }
            Message::PreviewBackupStart => {
                if self.operation.is_some() {
                    return Command::none();
                }
                self.config.save();
                self.operation = Some(OngoingOperation::PreviewBackup);
                self.backup_screen.total_games = 0;
                self.backup_screen.log.entries.clear();

                std::env::set_current_dir(app_dir()).unwrap();

                let mut commands: Vec<Command<Message>> = vec![];
                for key in self.manifest.0.iter().map(|(k, _)| k.clone()) {
                    let game = self.manifest.0[&key].clone();
                    let roots = self.config.roots.clone();
                    let key2 = key.clone();
                    let steam_id = game.steam.clone().unwrap_or(SteamMetadata { id: None }).id;
                    commands.push(Command::perform(
                        async move {
                            scan_game_for_backup(&game, &key, &roots, &app_dir().to_string_lossy(), &steam_id)
                        },
                        move |info| Message::BackupStep {
                            game: key2.clone(),
                            info,
                        },
                    ));
                }

                commands.push(Command::perform(async move {}, move |_| Message::Idle));
                Command::batch(commands)
            }
            Message::RestoreStart => {
                if self.operation.is_some() {
                    return Command::none();
                }

                self.restore_screen.total_games = 0;
                self.restore_screen.log.entries.clear();
                self.modal_theme = None;

                let restore_path = crate::path::normalize(&self.config.restore.path);
                if !crate::path::is_dir(&restore_path) {
                    self.modal_theme = Some(ModalTheme::Error {
                        variant: Error::RestorationSourceInvalid { path: restore_path },
                    });
                    return Command::none();
                }

                self.config.save();
                self.operation = Some(OngoingOperation::Restore);

                let mut commands: Vec<Command<Message>> = vec![];
                for key in self.manifest.0.iter().map(|(k, _)| k.clone()) {
                    let source = restore_path.clone();
                    let key2 = key.clone();
                    commands.push(Command::perform(
                        async move {
                            let info = scan_game_for_restoration(&key, &source);
                            restore_game(&info);
                            info
                        },
                        move |info| Message::RestoreStep {
                            game: key2.clone(),
                            info,
                        },
                    ));
                }

                commands.push(Command::perform(async move {}, move |_| Message::Idle));
                Command::batch(commands)
            }
            Message::PreviewRestoreStart => {
                if self.operation.is_some() {
                    return Command::none();
                }

                self.restore_screen.total_games = 0;
                self.restore_screen.log.entries.clear();

                let restore_path = crate::path::normalize(&self.config.restore.path);
                if !crate::path::is_dir(&restore_path) {
                    self.modal_theme = Some(ModalTheme::Error {
                        variant: Error::RestorationSourceInvalid { path: restore_path },
                    });
                    return Command::none();
                }

                self.config.save();
                self.operation = Some(OngoingOperation::PreviewRestore);

                let mut commands: Vec<Command<Message>> = vec![];
                for key in self.manifest.0.iter().map(|(k, _)| k.clone()) {
                    let source = restore_path.clone();
                    let key2 = key.clone();
                    commands.push(Command::perform(
                        async move { scan_game_for_restoration(&key, &source) },
                        move |info| Message::RestoreStep {
                            game: key2.clone(),
                            info,
                        },
                    ));
                }

                commands.push(Command::perform(async move {}, move |_| Message::Idle));
                Command::batch(commands)
            }
            Message::BackupStep { game, info } => {
                if !info.found_files.is_empty() || !info.found_registry_keys.is_empty() {
                    self.backup_screen.total_games += 1;
                    self.backup_screen.log.entries.push(GameListEntry {
                        name: game,
                        files: info.found_files,
                        registry_keys: info.found_registry_keys,
                    });
                }
                Command::none()
            }
            Message::RestoreStep { game, info } => {
                if !info.found_files.is_empty() || !info.found_registry_keys.is_empty() {
                    self.restore_screen.total_games += 1;
                    self.restore_screen.log.entries.push(GameListEntry {
                        name: game,
                        files: info.found_files,
                        registry_keys: info.found_registry_keys,
                    });
                }
                Command::none()
            }
            Message::EditedBackupTarget(text) => {
                self.config.backup.path = text;
                Command::none()
            }
            Message::EditedRestoreSource(text) => {
                self.config.restore.path = text;
                Command::none()
            }
            Message::EditedRootPath(index, path) => {
                self.config.roots[index].path = path;
                Command::none()
            }
            Message::EditedRootStore(index, store) => {
                self.config.roots[index].store = store;
                Command::none()
            }
            Message::AddRoot => {
                self.backup_screen
                    .root_editor
                    .rows
                    .push((button::State::default(), text_input::State::default()));
                self.config.roots.push(RootsConfig {
                    path: "".into(),
                    store: Store::Other,
                });
                Command::none()
            }
            Message::RemoveRoot(index) => {
                self.backup_screen.root_editor.rows.remove(index);
                self.config.roots.remove(index);
                Command::none()
            }
            Message::SwitchScreenToBackup => {
                self.screen = Screen::Backup;
                Command::none()
            }
            Message::SwitchScreenToRestore => {
                self.screen = Screen::Restore;
                Command::none()
            }
        }
    }

    fn view(&mut self) -> Element<Message> {
        if let Some(m) = &self.modal_theme {
            return self.modal.view(m, &self.translator, &self.config).into();
        }

        match self.screen {
            Screen::Backup => self
                .backup_screen
                .view(&self.config, &self.translator, self.operation.is_none()),
            Screen::Restore => self
                .restore_screen
                .view(&self.config, &self.translator, self.operation.is_none()),
        }
        .into()
    }
}

mod style {
    use iced::{button, container, Background, Color, Vector};

    pub enum Button {
        Primary,
        Disabled,
        Negative,
        Navigation,
    }
    impl button::StyleSheet for Button {
        fn active(&self) -> button::Style {
            button::Style {
                background: match self {
                    Button::Primary => Some(Background::Color(Color::from_rgb8(28, 107, 223))),
                    Button::Disabled => Some(Background::Color(Color::from_rgb8(169, 169, 169))),
                    Button::Negative => Some(Background::Color(Color::from_rgb8(255, 0, 0))),
                    Button::Navigation => Some(Background::Color(Color::from_rgb8(136, 0, 219))),
                },
                border_radius: 4,
                shadow_offset: Vector::new(1.0, 1.0),
                text_color: Color::from_rgb8(0xEE, 0xEE, 0xEE),
                ..button::Style::default()
            }
        }

        fn hovered(&self) -> button::Style {
            button::Style {
                text_color: Color::WHITE,
                shadow_offset: Vector::new(1.0, 2.0),
                ..self.active()
            }
        }
    }

    pub enum Container {
        GameListEntry,
        GameListEntryTitle,
        GameListEntryBody,
    }

    impl container::StyleSheet for Container {
        fn style(&self) -> container::Style {
            container::Style {
                background: match self {
                    Container::GameListEntryTitle => Some(Background::Color(Color::from_rgb8(230, 230, 230))),
                    _ => None,
                },
                border_color: match self {
                    Container::GameListEntry => Color::from_rgb8(230, 230, 230),
                    _ => Color::BLACK,
                },
                border_width: match self {
                    Container::GameListEntry => 1,
                    _ => 0,
                },
                border_radius: match self {
                    Container::GameListEntry | Container::GameListEntryTitle => 10,
                    _ => 0,
                },
                ..container::Style::default()
            }
        }
    }
}

pub fn run_gui() {
    App::run(iced::Settings::default())
}