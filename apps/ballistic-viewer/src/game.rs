//! The game around the levels, like `base.cmo`'s event handler and the
//! menus' scripts (research findings 25): the main menu over the menu
//! level, the level select, pausing, the end of a level, game over.
//!
//! | Menu | Buttons |
//! |---|---|
//! | Main | Start (level select), Highscore, Options, Credits, Exit (asks) |
//! | Level select | the open levels, Back |
//! | Pause (Escape) | Restart (asks), Highscore, Options, Exit level (asks), Back |
//! | End (after the score) | Restart (asks), Highscore, Options, Main menu, Next level |
//! | Game over | Restart, Main menu |
//! | Options | Graphics, Controls, Sound (not yet), Back |
//!
//! The highscore table opened from a menu starts at the current level (the
//! first from the main menu) and goes back to that menu. Credits aren't
//! there yet. The window carries out the [`Action`]s.

use ballistic_bundle::{LevelFlow, Scene};
use ballistic_render::Camera;

use crate::flow::EventSounds;
use crate::highscore_screens::{self, HighscoreScreens};
use crate::menus::{self, MenuId, MenuScreens};
use crate::play::AfterScore;
use crate::save::SharedSave;

/// What the window is to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Load this level (its scene key) and play it.
    Play(String),
    /// The current level again.
    Restart,
    /// Back to the main menu over the menu level.
    MainMenu,
    Quit,
}

pub struct Game {
    pub menus: MenuScreens,
    pub tables: HighscoreScreens,
    sounds: EventSounds,
    save: SharedSave,
    levels: usize,
    /// The level being played (0-based), if any.
    level: Option<usize>,
    /// After the level's score: what comes next.
    next: Option<AfterScore>,
    /// Where the table or options go back to.
    back_to: Option<MenuId>,
    /// The question being asked, and the menu that asked it.
    question: Option<(&'static str, MenuId)>,
    actions: Vec<Action>,
}

/// The scene the main menu shows.
pub const MENU_LEVEL: &str = "menulevel";

pub fn level_key(level: usize) -> String {
    format!("level/level_{:02}", level + 1)
}

impl Game {
    /// The game for the bundle's menus, at the main menu.
    pub fn create(
        gpu: &ballistic_render::Gpu,
        bundle: &mut crate::play::Bundle,
        format: wgpu::TextureFormat,
        save: SharedSave,
    ) -> anyhow::Result<Self> {
        let physics = bundle.physics()?;
        let menus = MenuScreens::new(gpu, bundle, &physics.menus, format, "english")?;
        let tables = HighscoreScreens::new(
            gpu,
            bundle,
            &physics.highscores,
            &physics.menus.fonts,
            format,
            "english",
        )?;
        let mut g = Self::new(
            menus,
            tables,
            &physics.flow,
            physics.highscores.defaults.len(),
            save,
        );
        g.main_menu();
        Ok(g)
    }

    /// `levels`: how many there are; `flow`: the event sounds.
    pub fn new(
        menus: MenuScreens,
        tables: HighscoreScreens,
        flow: &LevelFlow,
        levels: usize,
        save: SharedSave,
    ) -> Self {
        Self {
            menus,
            tables,
            sounds: EventSounds::new(flow),
            save,
            levels,
            level: None,
            next: None,
            back_to: None,
            question: None,
            actions: Vec::new(),
        }
    }

    /// A menu or table is up: the level waits.
    pub fn paused(&self) -> bool {
        self.menus.active() || self.tables.active()
    }

    /// At the main menu (no level).
    pub fn main_menu(&mut self) {
        self.level = None;
        self.next = None;
        self.menus.open(MenuId::Main, Vec::new());
    }

    /// A level started (0-based).
    pub fn playing(&mut self, level: Option<usize>) {
        self.level = level;
        self.next = None;
        self.menus.close();
    }

    /// Escape during play: the pause menu.
    pub fn pause(&mut self) {
        if !self.paused() {
            self.sounds.emit("Menu_Click");
            self.menus.open(MenuId::Pause, Vec::new());
        }
    }

    /// The last life is gone: the game over menu.
    pub fn dead(&mut self) {
        self.menus.open(MenuId::Dead, Vec::new());
    }

    /// The level's score screens are done: the end menu.
    pub fn level_done(&mut self, next: AfterScore) {
        self.next = Some(next);
        self.menus.open(MenuId::End, Vec::new());
    }

    pub fn key(&mut self, scan: u32) -> bool {
        let taken = if self.tables.active() {
            self.tables.key(scan)
        } else {
            self.menus.key(scan)
        };
        self.handle();
        taken
    }

    pub fn key_up(&mut self, scan: u32) {
        self.tables.key_up(scan);
    }

    pub fn point(&mut self, pos: (f32, f32), size: (u32, u32)) {
        self.menus.point(pos, size);
    }

    pub fn click(&mut self, pos: (f32, f32), size: (u32, u32)) -> bool {
        let taken = self.menus.click(pos, size);
        self.handle();
        taken
    }

    /// Sounds to play (bundle key, volume) after `real_dt`.
    pub fn sounds(&mut self, real_dt: f32) -> Vec<(String, f32)> {
        self.sounds.frame(real_dt)
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    fn handle(&mut self) {
        for e in self.tables.take_events() {
            match e {
                highscore_screens::Event::Message(m) => self.sounds.emit(m),
                highscore_screens::Event::Left => self.back(),
                highscore_screens::Event::Named(_) => {}
            }
        }
        for e in self.menus.take_events() {
            match e {
                menus::Event::Message(m) => self.sounds.emit(m),
                menus::Event::Picked(id, i) => self.picked(id, i),
                menus::Event::Answered(yes) => self.answered(yes),
            }
        }
    }

    /// Back to the menu that opened the table or options.
    fn back(&mut self) {
        let to = self.back_to.take().unwrap_or(MenuId::Main);
        self.open(to);
    }

    fn open(&mut self, id: MenuId) {
        let locked = if id == MenuId::Start {
            let save = self.save.borrow();
            (0..self.levels)
                .map(|i| !save.data.is_unlocked(i))
                .chain([false])
                .collect()
        } else {
            Vec::new()
        };
        self.menus.open(id, locked);
    }

    fn table(&mut self, from: MenuId) {
        self.back_to = Some(from);
        self.tables
            .start_table(self.level.unwrap_or(0), self.levels, false);
    }

    fn options(&mut self, from: MenuId) {
        // Options from options keeps where they came from.
        if from != MenuId::Options {
            self.back_to = Some(from);
        }
        self.menus.open(MenuId::Options, Vec::new());
    }

    fn ask(&mut self, question: &'static str, from: MenuId) {
        self.question = Some((question, from));
        self.menus.ask(question);
    }

    fn picked(&mut self, id: MenuId, i: usize) {
        match (id, i) {
            (MenuId::Main, 0) => self.open(MenuId::Start),
            (MenuId::Main, 1) => self.table(MenuId::Main),
            (MenuId::Main, 2) => self.options(MenuId::Main),
            (MenuId::Main, 3) => {
                eprintln!("credits: not there yet");
                self.open(MenuId::Main);
            }
            (MenuId::Main, _) => self.ask("Exit Game?", MenuId::Main),
            (MenuId::Start, i) if i < self.levels => {
                self.sounds.emit("Menu_Load");
                self.actions.push(Action::Play(level_key(i)));
            }
            (MenuId::Start, _) => self.open(MenuId::Main),
            (MenuId::Pause | MenuId::End, 0) => self.ask("Restart Level?", id),
            (MenuId::Pause | MenuId::End, 1) => self.table(id),
            (MenuId::Pause | MenuId::End, 2) => self.options(id),
            (MenuId::Pause, 3) => self.ask("Exit Level?", MenuId::Pause),
            // Back to the level.
            (MenuId::Pause, _) => self.sounds.emit("Unpause Level"),
            (MenuId::End, 3) | (MenuId::Dead, 1) => self.actions.push(Action::MainMenu),
            (MenuId::End, _) => match self.next.take() {
                Some(AfterScore::Level(key)) => self.actions.push(Action::Play(key)),
                _ => self.actions.push(Action::MainMenu),
            },
            (MenuId::Dead, _) => self.actions.push(Action::Restart),
            (MenuId::Options, 3) => self.back(),
            (MenuId::Options, _) => {
                eprintln!("options: not there yet");
                self.menus.open(MenuId::Options, Vec::new());
            }
        }
    }

    fn answered(&mut self, yes: bool) {
        let Some((question, from)) = self.question.take() else {
            return;
        };
        match (question, yes) {
            ("Exit Game?", true) => self.actions.push(Action::Quit),
            ("Restart Level?", true) => self.actions.push(Action::Restart),
            ("Exit Level?", true) => self.actions.push(Action::MainMenu),
            _ => self.open(from),
        }
    }
}

/// The menu level's camera: `Cam_MenuLevel`, looking at its target.
pub fn menu_camera(scene: &Scene, base: Camera) -> Camera {
    let at = |name: &str| {
        scene.objects.iter().find(|o| o.name == name).map(|o| {
            glam::Mat4::from_cols_array_2d(&o.transform).transform_point3(glam::Vec3::ZERO)
        })
    };
    let (Some(eye), Some(target)) = (at("Cam_MenuLevel"), at("Cam_MenuLevel_Target")) else {
        return base;
    };
    let mut cam = Camera {
        position: eye,
        ..base
    };
    cam.look_at(target);
    cam
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::SaveFile;
    use ballistic_bundle as bb;

    const ENTER: u32 = 28;
    const ESCAPE: u32 = 1;
    const DOWN: u32 = 208;
    const LEFT: u32 = 203;

    fn menu(n: usize, first: usize, escape: Option<usize>) -> bb::ButtonMenu {
        bb::ButtonMenu {
            buttons: vec![bb::MenuButton::default(); n],
            first,
            escape,
            ..bb::ButtonMenu::default()
        }
    }

    /// The menus as the game has them (button counts, first and escape
    /// buttons), three levels with the first open.
    fn game() -> Game {
        let menus = bb::Menus {
            main: menu(5, 0, Some(4)),
            start: menu(4, 0, Some(3)),
            pause: menu(5, 4, Some(4)),
            end: menu(5, 4, Some(4)),
            dead: menu(2, 0, None),
            options: menu(4, 0, Some(3)),
            ..bb::Menus::default()
        };
        let mut highscores = bb::Highscores {
            defaults: vec![Vec::new(); 3],
            unlocked: vec![true, false, false],
            ..bb::Highscores::default()
        };
        highscores.table.leave_keys = vec![1, 28];
        let flow = bb::LevelFlow {
            new_ball_ms: 0.0,
            lost_ms: 0.0,
            depth_cubes: String::new(),
            prop_depth_margin: 0.0,
            prop_depth_groups: Vec::new(),
            event_sounds: Vec::new(),
        };
        let save = SaveFile::in_memory(&highscores).shared();
        Game::new(
            MenuScreens::with_textures(&menus, "english", std::collections::HashMap::new()),
            HighscoreScreens::with_textures(
                &highscores,
                &std::collections::BTreeMap::new(),
                "english",
                std::collections::HashMap::new(),
            ),
            &flow,
            3,
            save,
        )
    }

    #[test]
    fn start_plays_an_open_level() {
        let mut g = game();
        g.main_menu();
        assert!(g.key(ENTER), "start");
        assert_eq!(g.menus.showing(), Some(MenuId::Start));
        // Level 2 is closed: Down skips to Back.
        assert!(g.key(ENTER));
        assert_eq!(g.take_actions(), [Action::Play("level/level_01".into())]);
    }

    #[test]
    fn exit_asks_first() {
        let mut g = game();
        g.main_menu();
        assert!(g.key(ESCAPE));
        assert!(g.key(ENTER), "no");
        assert_eq!(g.menus.showing(), Some(MenuId::Main));
        assert!(g.key(ESCAPE));
        assert!(g.key(LEFT));
        assert!(g.key(ENTER), "yes");
        assert_eq!(g.take_actions(), [Action::Quit]);
    }

    #[test]
    fn pause_goes_back_and_restarts_after_asking() {
        let mut g = game();
        g.playing(Some(0));
        g.pause();
        assert!(g.paused());
        assert_eq!(g.menus.showing(), Some(MenuId::Pause));
        assert!(g.key(ESCAPE), "back");
        assert!(!g.paused());
        g.pause();
        // Starts on Back; Down goes round to Restart.
        assert!(g.key(DOWN));
        assert!(g.key(ENTER));
        assert!(g.key(LEFT));
        assert!(g.key(ENTER));
        assert_eq!(g.take_actions(), [Action::Restart]);
    }

    #[test]
    fn the_end_menu_goes_on_and_the_table_comes_back() {
        let mut g = game();
        g.playing(Some(0));
        g.level_done(AfterScore::Level("level/level_02".into()));
        assert_eq!(g.menus.showing(), Some(MenuId::End));
        assert!(g.key(DOWN));
        assert!(g.key(DOWN));
        assert!(g.key(ENTER), "highscore");
        assert!(g.tables.active());
        assert!(g.key(ESCAPE));
        assert_eq!(g.menus.showing(), Some(MenuId::End));
        // Escape picks Next level (the end menu's escape button).
        assert!(g.key(ESCAPE));
        assert_eq!(g.take_actions(), [Action::Play("level/level_02".into())]);
    }
}
