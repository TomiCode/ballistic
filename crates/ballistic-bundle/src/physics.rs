//! Physics data extracted from the game's scripts and tables (`data/physics.json`).
//! Bundle space: right-handed, Y up, original units.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsData {
    /// World gravity during play.
    pub gravity: [f32; 3],
    /// Simulated seconds per real second during play.
    pub time_factor: f32,
    /// Player ball types (`Physicalize_GameBall`).
    pub balls: Vec<BallParams>,
    /// Static level surfaces by level group (`Physicalize_Floors`).
    pub surfaces: Vec<SurfaceParams>,
    /// Movable level props by prefab group (`Physicalize_Convex`, `Physicalize_Balls`).
    pub props: Vec<PropParams>,
    /// Moving elements (`P_Modul_*` prefabs): bodies and joints from their scripts.
    pub modules: Vec<ModuleSetup>,
    /// Ball transformers (`Gameplay_Ingame > Trafo Manager`).
    pub transformers: TransformerParams,
    /// The ball's hit and roll sounds (`Sound.nmo`'s `Sound_Manager`).
    pub sounds: BallSounds,
    /// Points, lives and score (`Gameplay_Energy`, the pickups).
    pub energy: EnergyParams,
    /// The level flow and the event sounds.
    pub flow: LevelFlow,
    /// The in-game HUD: points and lives.
    pub hud: Hud,
    /// Sky, light and background.
    pub environment: Environment,
    /// The prefabs' particle systems.
    pub particles: Vec<ParticleEmitter>,
    /// After the finish: the wait, the sky's fade.
    pub level_end: LevelEnd,
    /// The score screen after a level.
    pub score_screen: ScoreScreen,
    /// The highscore tables, the name entry and the table screen.
    pub highscores: Highscores,
    /// The menus.
    pub menus: Menus,
}

/// The menus (`Menu.nmo`). Keys are `DirectInput` scan codes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Menus {
    /// The menu fonts, by name (`GameFont_01`, …).
    pub fonts: std::collections::BTreeMap<String, HudFont>,
    /// A button's look: up, the active one, a locked one (texture key,
    /// colour).
    pub button_up: (Option<String>, [f32; 4]),
    pub button_over: (Option<String>, [f32; 4]),
    pub button_inactive: (Option<String>, [f32; 4]),
    /// `Menu_Main`: start, highscores, options, credits, exit.
    pub main: ButtonMenu,
    /// `Menu_Start`: the levels, then back.
    pub start: ButtonMenu,
    /// `Menu_Pause`: restart, highscores, options, exit level, back.
    pub pause: ButtonMenu,
    /// `Menu_End`: restart, highscores, options, main menu, next level.
    pub end: ButtonMenu,
    /// `Menu_Dead`: restart, main menu.
    pub dead: ButtonMenu,
    /// `Menu_Options`: graphics, keys, sound, back.
    pub options: ButtonMenu,
    pub yes_no: YesNoMenu,
}

/// A list of buttons (`Menu_*_ShowHide`): Up and Down move through them
/// (round), Enter or a click picks one, Escape picks `escape`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ButtonMenu {
    /// Shown with it (the dark band, a title's sprite).
    pub sprites: Vec<HudSprite>,
    pub title: Option<MenuText>,
    pub buttons: Vec<MenuButton>,
    /// The button active at first.
    pub first: usize,
    /// The button Escape picks (none: Escape does nothing).
    pub escape: Option<usize>,
    /// The font of a locked button's text (the level select's).
    pub locked_font: Option<String>,
    /// The keys it reacts to.
    pub keys: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MenuButton {
    pub sprite: HudSprite,
    pub text: MenuText,
}

/// `Menu_YesNo`: a question, Yes and No. Left and Right choose, Enter or a
/// click confirms, Escape says No.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct YesNoMenu {
    pub sprites: Vec<HudSprite>,
    /// Where the question goes.
    pub text: MenuText,
    pub yes: MenuButton,
    pub no: MenuButton,
    /// The questions the menus ask, by their local's name (`Exit Game?`,
    /// `Restart Level?`, `Exit Level?`), each by language.
    pub questions: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
    pub keys: Vec<u32>,
}

/// Highscores (`base.cmo`'s *Highscore*, `Menu.nmo`'s
/// `Menu_HighscoreEntry` and `Menu_Highscore`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Highscores {
    /// Each level's table on a new install (`DB_Highscore_LvNN`), best
    /// first.
    pub defaults: Vec<Vec<HighscoreRow>>,
    /// Which levels are open on a new install (`DB_Levelfreischaltung`).
    pub unlocked: Vec<bool>,
    /// The name the entry starts with on a new install (`DB_Options`).
    pub default_name: String,
    pub entry: HighscoreEntryScreen,
    pub table: HighscoreTableScreen,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HighscoreRow {
    pub name: String,
    pub score: i32,
}

/// A menu's *2D Text*: in `rect` (screen fractions of 4:3), in font `font`,
/// aligned within `margins` (pixels at 480 lines). `text`: a fixed text by
/// language (`english`, `german`, …), or empty when the screen fills it in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MenuText {
    pub rect: [f32; 4],
    pub font: String,
    pub align: TextAlign,
    pub margins: [f32; 4],
    pub text: std::collections::BTreeMap<String, String>,
    /// Broken into lines at spaces to fit the box (`WordWrap`).
    pub wrap: bool,
}

/// `Menu_HighscoreEntry`: a new highscore's name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HighscoreEntryScreen {
    /// Shown while it's on (the black screen behind it stays until the
    /// table is left).
    pub sprites: Vec<HudSprite>,
    /// *New Highscore!*
    pub title: MenuText,
    /// The score, then this text (*Points*).
    pub score: MenuText,
    /// The name being typed, with a caret `caret_size` × the text height
    /// wide.
    pub name: MenuText,
    pub caret_size: f32,
    /// At most this many characters.
    pub max_len: u32,
    /// The *Ok* button's text; the key that confirms (scan codes).
    pub ok: MenuText,
    pub keys: Vec<u32>,
}

/// `Menu_Highscore`: one level's table at a time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HighscoreTableScreen {
    pub sprites: Vec<HudSprite>,
    /// The headline text, then the level's number.
    pub title: MenuText,
    /// The button leaving the table: *Next* after a level, `back_text`
    /// (*Back*) from a menu.
    pub back: MenuText,
    pub back_text: std::collections::BTreeMap<String, String>,
    /// Each place's name (left, moved right by `name_offset`, a screen
    /// fraction) and score (right).
    pub names: Vec<MenuText>,
    pub scores: Vec<MenuText>,
    pub name_offset: f32,
    /// The previous / next level arrows (sprite names), shown in
    /// `arrow_pressed` (texture key, colour) while their key is down.
    pub arrows: [String; 2],
    pub arrow_pressed: (Option<String>, [f32; 4]),
    /// Scan codes: previous level, next level, leave.
    pub previous_keys: Vec<u32>,
    pub next_keys: Vec<u32>,
    pub leave_keys: Vec<u32>,
}

/// The level's end (`Gameplay_Events` after `Level_Finish`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LevelEnd {
    /// The score screen comes this long after the finish; after the last
    /// level, `last_wait_ms`.
    pub wait_ms: f32,
    pub last_wait_ms: f32,
    /// *fadeout Sky*: the cloud layer's colour goes from `sky_fade[0]` to
    /// `sky_fade[1]` over `sky_fade_ms`.
    pub sky_fade_ms: f32,
    pub sky_fade: [[f32; 4]; 2],
    /// The camera's far clip from the finish on.
    pub far_clip: f32,
    /// Keys that cut the wait short (`DirectInput` scan codes).
    pub skip_keys: Vec<u32>,
}

/// The score screen (`Menu.nmo`'s `Menu_Score`). Rectangles are fractions of
/// the screen (left, top, right, bottom).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScoreScreen {
    /// The field, the highlight and the line, at their full colours (they
    /// fade in from transparent over `fade_ms`, and out again).
    pub sprites: Vec<HudSprite>,
    pub fade_ms: f32,
    /// The highlight sprite; its top-left for each row, and with fast
    /// scoring.
    pub highlight: String,
    pub highlight_rows: Vec<[f32; 2]>,
    pub highlight_fast: [f32; 2],
    pub font: HudFont,
    pub shadow: TextShadow,
    /// Level bonus, points, extra lives, total.
    pub rows: Vec<ScoreRow>,
    pub align: TextAlign,
    /// Left, top, right, bottom, in pixels.
    pub margins: [f32; 4],
    /// Between the steps.
    pub step_ms: f32,
    /// The points are counted over `step` a frame while the count is below
    /// `limit`, row by row.
    pub count_speed: Vec<[i32; 2]>,
    /// The counter sound's pitch for each `count_speed` row: from, to (over
    /// `pitch_ramp_ms`).
    pub count_pitch: Vec<[f32; 2]>,
    pub pitch_ramp_ms: f32,
    /// Bundle sound key.
    pub counter_sound: String,
    /// One life counted every `life_ms`; with fast scoring, taken every
    /// `fast_life_ms`.
    pub life_ms: f32,
    pub fast_life_ms: f32,
    /// The total shows this long (or until a key) before the screen fades.
    pub wait_ms: f32,
    /// Keys for fast scoring and for leaving (`DirectInput` scan codes).
    pub keys: Vec<u32>,
}

/// A text's drop shadow (*Set Font Properties*).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TextShadow {
    pub color: [f32; 4],
    /// Radians, and pixels at 480 lines.
    pub angle: f32,
    pub distance: f32,
    /// The shadow glyphs' size, relative to the text's.
    pub size: [f32; 2],
}

/// One score screen row: its label (by language: `english`, `german`, …) and
/// value rectangles.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScoreRow {
    pub label: std::collections::BTreeMap<String, String>,
    pub label_rect: [f32; 4],
    pub value_rect: [f32; 4],
}

/// Points, lives and score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnergyParams {
    /// A level starts with these.
    pub start_points: i32,
    pub start_lives: i32,
    /// One point less every this long (real time, while the counter runs).
    pub tick_ms: f32,
    /// The score at the end: the level's bonus + the points left + the lives
    /// left × this.
    pub life_bonus: i32,
    /// Each level's bonus (`AllLevel`), in level order.
    pub level_bonus: Vec<i32>,
    pub extra_life: ExtraLife,
    pub extra_point: ExtraPoint,
}

/// `P_Extra_Life`: one life more when the ball comes within `reach`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtraLife {
    /// Prefab group.
    pub group: String,
    pub reach: WakeParams,
    pub look: ExtraLifeLook,
}

/// How an extra life looks (`P_Extra_Life_MF Script`, `P_Extra_Life Anim
/// Script`), by prefab object name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtraLifeLook {
    /// Shown while the ball is within this (X and Y) of it.
    pub show_distance: f32,
    pub sphere: String,
    /// A sprite, the sphere's child.
    pub ball: String,
    pub shadow: String,
    /// The shadow's material, whose emissive colour follows the bounce.
    pub shadow_material: String,
    /// Bouncing (only over a floor): the sphere's position, relative to the
    /// prefab's frame, from `bounce_from` to `bounce_to` by the curve.
    pub bounce_from: [f32; 3],
    pub bounce_to: [f32; 3],
    pub bounce: Progression,
    pub shadow_from: [f32; 4],
    pub shadow_to: [f32; 4],
    /// Squashing: the sphere's scale, always.
    pub squash_from: [f32; 3],
    pub squash_to: [f32; 3],
    pub squash: Progression,
    /// The floor test: a ray from `floor_ray_origin` (in the frame) down
    /// `floor_ray_depth`, against the level group `floor_group`.
    pub floor_ray_origin: [f32; 3],
    pub floor_ray_depth: f32,
    pub floor_group: String,
    /// `TT_TextureSine` on the sphere's texture coordinates.
    pub uv_wave: UvWave,
}

/// `TT_TextureSine`: each vertex's (u, v) moves towards (0.5, 0.5) by
/// cos and sin of time x `velocity` + (index / count - 0.5) x `spread`,
/// scaled by `amplitude`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct UvWave {
    pub amplitude: [f32; 2],
    pub velocity: f32,
    pub spread: f32,
}

/// `P_Extra_Point` (`TT Extra`): `activation_points` when the ball comes
/// within `activation_distance` of it. Then its small balls fly off (`away_*`
/// for `away_ms`) and chase the ball (`force`, `damping`, varied by
/// `force_width` per ball); each that comes within `hit_distance` is
/// `hit_points` more.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtraPoint {
    pub group: String,
    pub small_balls: u32,
    pub activation_distance: f32,
    pub hit_distance: f32,
    pub away_force: f32,
    pub away_damping: f32,
    pub away_ms: f32,
    pub force: f32,
    pub damping: f32,
    pub force_width: f32,
    pub activation_points: i32,
    pub hit_points: i32,
    /// Where the hits show (a group of the prefab).
    pub hit_frames: String,
    pub look: ExtraPointLook,
}

/// How an extra point looks (`P_Extra_Point_MF Script`, *TT Extra*), by
/// prefab object name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtraPointLook {
    /// *TT Extra* is on (shown) while the ball is within this.
    pub show_distance: f32,
    /// The small balls circle at this rate (radians per second).
    pub rotation_speed: f32,
    /// The centre ball sprite (`Ball0`), hidden at the activation.
    pub centre: String,
    /// The small ball sprites, in *TT Extra*'s order; each has a child frame
    /// with its trail.
    pub balls: Vec<String>,
    /// The floor glow, shown until the activation unless no floor is below.
    pub floor: String,
    /// The floor test (a *Flying Extra?* has none below): a ray from
    /// `floor_ray_origin` (in the frame) down `floor_ray_depth`, against the
    /// level group `floor_group`.
    pub floor_ray_origin: [f32; 3],
    pub floor_ray_depth: f32,
    pub floor_group: String,
}

/// The ball's hit and roll sounds. Hit IDs: 1 stone, 2 wood, 3 metal, 4 dome;
/// roll IDs: 1 stone, 2 wood, 3 metal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BallSounds {
    /// Per ball type (`BallSound`): bundle sound keys.
    pub sets: Vec<BallSoundSet>,
    /// *Hit Sounds*: one collision test per hit ID, in ID order.
    pub hits: Vec<HitTest>,
    /// `PhysicsContinuousContact`: a roll ID's sound starts after this long
    /// in contact, and stops after this long without (simulated seconds).
    pub roll_delay_start_s: f32,
    pub roll_delay_end_s: f32,
    /// `MultiRollSoundControl`: the roll sounds' volume is speed × this, with
    /// the ball's speed in units per real second.
    pub roll_volume: f32,
    /// ... and their pitch `[a, b]`: a + speed × b.
    pub roll_pitch: [f32; 2],
    /// Level groups whose members get a hit or roll ID (level init).
    pub hit_groups: Vec<SoundGroup>,
    pub roll_groups: Vec<SoundGroup>,
    /// Prefab objects' IDs (their attributes).
    pub objects: Vec<ObjectSoundIds>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BallSoundSet {
    /// Ball type name, e.g. `Ball_Wood`.
    pub ball: String,
    /// Roll sounds for roll IDs 1, 2, 3.
    pub roll: Vec<String>,
    /// Hit sounds for hit IDs 1, 2, 3, 4.
    pub hit: Vec<String>,
}

/// A `PhysicsCollDetection` on the ball for one hit ID: hits faster than
/// `min_speed` play at `min(speed / max_speed, 1)`, then it waits `sleep_s`
/// (simulated time).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HitTest {
    pub id: u8,
    pub min_speed: f32,
    pub max_speed: f32,
    pub sleep_s: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundGroup {
    pub group: String,
    pub id: u8,
}

/// An object's hit and roll IDs (0: none).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectSoundIds {
    /// Prefab scene key, e.g. `ph/p_box`.
    pub scene: String,
    pub object: String,
    pub hit: u8,
    pub roll: u8,
}

/// How a transformer swaps the player's ball (`Trafo Manager` in `Gameplay.nmo`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformerParams {
    /// Transformer prefab groups and the ball each one turns the ball into.
    pub types: Vec<TransformerType>,
    /// The ball's centre must come nearer than this to a transformer's origin
    /// (and the ball must be of another type).
    pub reach: f32,
    /// The pull into the transformer: *TT Set Dynamic Position* in the
    /// transformer's frame, per axis (bundle space). The ball settles at
    /// `−offset` in that frame.
    pub pull_force: [f32; 3],
    pub pull_damping: [f32; 3],
    pub pull_offset: [f32; 3],
    /// How long the pull runs.
    pub pull_ms: f32,
    /// Then the ball hangs there this long until it bursts.
    pub hold_ms: f32,
    /// Then this long until the new ball replaces it.
    pub swap_ms: f32,
    /// The transformer's animation (`AnimTrafo`).
    pub animation: TransformerAnimation,
    /// The old ball's pieces, one set per ball type.
    pub pieces: Vec<BallPieces>,
    /// A ball type's pieces fade out this long after a transformation from it
    /// (or at once if the ball transforms from that type again first).
    pub pieces_fade_after_ms: f32,
    /// The sound the trigger starts (through the sound scripts).
    pub sound: Option<SoundCue>,
}

/// A sound played some frames and milliseconds after an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundCue {
    /// Bundle sound key, e.g. `misc_trafo`.
    pub sound: String,
    pub delay_frames: u32,
    pub delay_ms: f32,
}

/// The level flow (`Gameplay.nmo`'s `BallManager`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelFlow {
    /// *New Ball*: the ball comes (and control with it) this long after the
    /// level starts or a ball is lost; the lightning plays meanwhile.
    pub new_ball_ms: f32,
    /// *Deactivate Ball*: a lost ball lingers this long before it's taken
    /// away.
    pub lost_ms: f32,
    /// The ball is lost when its box meets one of this level group's boxes.
    pub depth_cubes: String,
    /// Props of `prop_depth_groups` are taken away this far below the lowest
    /// cube.
    pub prop_depth_margin: f32,
    pub prop_depth_groups: Vec<String>,
    /// What the sound scripts play for each message (by name).
    pub event_sounds: Vec<EventSound>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventSound {
    /// The message's name, e.g. `Checkpoint reached`.
    pub message: String,
    pub cues: Vec<SoundCue>,
}

/// A particle system on a prefab entity (*Point*, *Planar* or *Spherical
/// Particle System*). Times in milliseconds, speeds in units per
/// millisecond. Each `variance` is ± (uniform), 0 when not enabled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleEmitter {
    /// The prefab scene and the entity the system is on.
    pub scene: String,
    pub object: String,
    pub shape: EmitterShape,
    pub render: ParticleRender,
    /// Texture key; `None`: untextured.
    pub texture: Option<String>,
    pub blend: (crate::BlendFactor, crate::BlendFactor),
    /// Time between emissions.
    pub delay_ms: f32,
    /// Spread around the emission direction (radians).
    pub yaw_variance: f32,
    pub pitch_variance: f32,
    pub speed: Varied,
    pub lifespan_ms: Varied,
    /// At most this many alive.
    pub max: u32,
    /// Particles per emission.
    pub emission: Varied,
    pub size: Varied,
    /// `None`: the size stays.
    pub end_size: Option<Varied>,
    pub color: [f32; 4],
    pub color_variance: [f32; 4],
    /// `None`: the colour stays.
    pub end_color: Option<[f32; 4]>,
    pub end_color_variance: [f32; 4],
    /// The texture's frames (a grid; 1 = the whole texture).
    pub frames: u32,
    /// Steps by the real frame time; otherwise by `delta_ms`.
    pub real_time: bool,
    pub delta_ms: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Varied {
    pub value: f32,
    pub variance: f32,
}

/// Where particles start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmitterShape {
    /// At the entity, along its Z axis (spread by yaw and pitch).
    Point,
    /// On the entity's local XY square (−1..1), along its Z axis.
    Planar,
    /// On the entity's unit sphere, outwards.
    Spherical,
}

/// How particles are drawn (*Particle Rendering*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticleRender {
    /// One camera-facing triangle per particle (apex up).
    FastSprite,
    Sprite,
    OrientableSprite,
    Line,
    /// Another mode (its number), not drawn yet.
    Other(u32),
}

/// Sky, light and background during play (`Gameplay_Sky`, `Light_Ingame`,
/// the scene's settings). There's no fog in play.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    /// Behind everything (`Set Background Color`), RGBA.
    pub background: [f32; 4],
    /// The scene's ambient light (RGB).
    pub ambient: [f32; 3],
    /// The direction `Light_Ingame` shines (bundle space, normalised). Its
    /// colour is the level's.
    pub light_direction: [f32; 3],
    pub sky: SkyParams,
    pub cloud_layer: CloudLayer,
    pub levels: Vec<LevelEnvironment>,
}

/// *TT Sky*: textured faces around the camera, following its position, not
/// its rotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkyParams {
    /// The faces' corners lie on a circle of this radius.
    pub radius: f32,
    /// The sky's field of view goes this fraction of the way to 180°.
    pub distortion: f32,
    /// The faces' height; `None` for square faces.
    pub height: Option<f32>,
    /// The centre's height relative to the camera.
    pub y: f32,
    pub color: [f32; 4],
    /// Texture key suffixes of the side faces, in order (e.g. `_back`).
    pub sides: Vec<String>,
    pub top: Option<String>,
    pub bottom: Option<String>,
}

/// A level's own environment, from `AllLevel`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelEnvironment {
    /// Scene key, e.g. `level/level_01`.
    pub level: String,
    /// The sky's texture key prefix, e.g. `sky/sky_l` (+ a face suffix).
    pub sky: String,
    /// `Light_Ingame`'s colour (RGBA).
    pub light: [f32; 4],
    /// The cloud layer's texture scroll per second.
    pub cloud_scroll: [f32; 2],
}

/// An optional cloud layer: the level object `object`, if it has one (`init
/// SkyLayer`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudLayer {
    pub object: String,
    /// Its vertex colour (lighting off).
    pub color: [f32; 4],
    /// Added to what's behind it (blend One / One).
    pub additive: bool,
    pub z_write: bool,
    /// On unless the player turns it off (`DB_Options`).
    pub enabled: bool,
}

/// The in-game HUD (`Camera.nmo`'s 2D entities, driven by `Gameplay_Energy`).
/// Rectangles are fractions of the screen (left, top, right, bottom).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hud {
    pub sprites: Vec<HudSprite>,
    pub font: HudFont,
    /// The points counter: text in this sprite's rectangle.
    pub points: HudText,
    pub lives: HudLives,
    /// The points' glow on an extra point: shown, faded in, hidden.
    pub glow: HudGlow,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HudSprite {
    pub name: String,
    /// Texture key, `None` for an untextured rectangle (the text's frame).
    pub texture: Option<String>,
    pub rect: [f32; 4],
    /// The part of the texture shown: left, top, right, bottom in UV.
    pub source: [f32; 4],
    /// Drawing order: higher on top.
    pub z_order: i32,
    /// The material's diffuse colour (RGBA), multiplied with the texture.
    pub color: [f32; 4],
    /// Shown with the HUD (`HUD_sprites`); the others are templates or
    /// shown by events.
    pub shown: bool,
}

/// A texture font (`TT CreateFontEx`, *Set Font Properties*).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HudFont {
    pub texture: String,
    /// By character code.
    pub glyphs: Vec<Glyph>,
    /// Glyph size: texture-coordinate size × screen size × this.
    pub scale: [f32; 2],
    /// `space[0]`: the gap between characters, in tenths of a space's
    /// advance. `space[1]`: extra line spacing.
    pub space: [f32; 2],
    pub color: [f32; 4],
}

/// One character of a texture font, in texture coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    pub u: f32,
    pub v: f32,
    pub width: f32,
    /// Space before and after the glyph.
    pub before: f32,
    pub after: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// A *2D Text* on a sprite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HudText {
    pub sprite: String,
    pub align: TextAlign,
    /// Left, top, right, bottom, in pixels.
    pub margins: [f32; 4],
}

/// The life balls: copies of `ball`, one per life, left of the sprite's own
/// place, with `end` left of the last.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HudLives {
    pub ball: String,
    pub end: String,
    /// The distance between balls (screen fraction).
    pub step: f32,
    /// A new ball's fade-in, after `end` has slid.
    pub fade_in: Progression,
    /// A lost ball's fade-out, before `end` slides back.
    pub fade_out: Progression,
    /// How `end` slides by one step.
    pub slide: Progression,
    /// From *Ball Off* to the ball's fade-out.
    pub lost_delay_ms: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HudGlow {
    pub sprite: String,
    pub fade: Progression,
}

/// The sounds of a ball type's pieces (`Ball_CollSound_Pieces`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PieceSounds {
    /// Bundle sound key, e.g. `pieces_wood`.
    pub sound: String,
    /// Pieces (objects of the `balls` scene) whose collisions play the sound,
    /// for `listen_ms` after the burst. Empty: the sound plays once at the
    /// burst.
    pub colliders: Vec<String>,
    /// `PhysicsCollDetection`: hits faster than `min_speed` play, at volume
    /// `min(speed / max_speed, 1)`; then that piece stays quiet for
    /// `sleep_s` of simulated time.
    pub min_speed: f32,
    pub max_speed: f32,
    pub sleep_s: f32,
    pub listen_ms: f32,
}

/// `AnimTrafo_MF Script` (`AnimTrafo.nmo`), played at the transformer that
/// transforms the ball. Object and material names refer to the `animtrafo`
/// scene; offsets are in bundle space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformerAnimation {
    /// The animation's root frame, put at the transformer's matrix.
    pub root: String,
    /// Shown in place of the transformer while the animation plays (with its
    /// children: the ring parts, their frames and the flash field).
    pub bars: String,
    /// The ring parts and the frames they move in.
    pub ring_parts: Vec<RingPart>,
    /// Opening moves each part to `ring_offset` × the curve in its frame.
    pub ring_offset: [f32; 3],
    pub ring_open: Progression,
    pub ring_close: Progression,
    /// The bars rise to `bars_offset` × the curve in the root frame.
    pub bars_offset: [f32; 3],
    pub lift: Progression,
    /// Meanwhile the transformer type's shadow material goes from the first
    /// diffuse colour to the second, by the same curve.
    pub shadow_colors: [[f32; 4]; 2],
    /// Shown while the bars move; its texture scrolls by the two offsets in
    /// turn, one every `flash_ms`.
    pub flash: String,
    pub flash_scroll: [[f32; 2]; 2],
    pub flash_ms: f32,
    /// Takes the diffuse colour (RGB) and emissive of the transformer mesh's
    /// second material.
    pub ring_material: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RingPart {
    pub object: String,
    pub frame: String,
}

/// A *Bezier Progression*: over `duration_ms`, a value from 0 to 1 shaped by
/// a curve.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Progression {
    pub duration_ms: f32,
    pub curve: Vec<CurvePoint>,
}

/// A control point of a 2D curve: position and Hermite tangents.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub position: [f32; 2],
    pub in_tangent: [f32; 2],
    pub out_tangent: [f32; 2],
}

impl Progression {
    /// The value after `elapsed_ms` (the curve's end value once it's over).
    pub fn value(&self, elapsed_ms: f32) -> f32 {
        let x = if self.duration_ms > 0.0 {
            (elapsed_ms / self.duration_ms).clamp(0.0, 1.0)
        } else {
            1.0
        };
        curve_y(&self.curve, x)
    }

    pub fn done(&self, elapsed_ms: f32) -> bool {
        elapsed_ms >= self.duration_ms
    }
}

/// A 2D curve's y at `x`: Hermite segments between the control points, with
/// each point's out tangent and the next one's in tangent. The parameter is
/// solved from x (Ballance's tangents make x linear in it).
pub fn curve_y(points: &[CurvePoint], x: f32) -> f32 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return x;
    };
    if x <= first.position[0] {
        return first.position[1];
    }
    if x >= last.position[0] {
        return last.position[1];
    }
    let Some(w) = points
        .windows(2)
        .find(|w| x >= w[0].position[0] && x <= w[1].position[0])
    else {
        return last.position[1];
    };
    let (p0, p1) = (w[0], w[1]);
    let hermite = |t: f32, a: f32, ma: f32, b: f32, mb: f32| {
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * a
            + (t3 - 2.0 * t2 + t) * ma
            + (-2.0 * t3 + 3.0 * t2) * b
            + (t3 - t2) * mb
    };
    let at = |t: f32, i: usize| {
        hermite(
            t,
            p0.position[i],
            p0.out_tangent[i],
            p1.position[i],
            p1.in_tangent[i],
        )
    };
    // x(t) is increasing: bisect for t.
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for _ in 0..32 {
        let mid = f32::midpoint(lo, hi);
        if at(mid, 0) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(f32::midpoint(lo, hi), 1)
}

/// One ball type's pieces (`Ball_Explosion_<Type>`, `Ball_ResetPieces_<Type>`
/// in `Balls.nmo`). Names refer to the `balls` scene; vectors are in bundle
/// space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BallPieces {
    /// Ball type name, e.g. `Ball_Wood`.
    pub ball: String,
    /// The group of piece objects, and the frame they're children of (put at
    /// the old ball's position).
    pub group: String,
    pub frame: String,
    /// The pieces' materials (faded out at the end).
    pub materials: Vec<String>,
    /// Physicalize: a convex body from each piece's own mesh. Mass and friction
    /// are drawn at random from these ranges for each piece (equal ends: fixed).
    pub mass: [f32; 2],
    pub friction: [f32; 2],
    pub elasticity: f32,
    pub linear_damping: f32,
    pub rotational_damping: f32,
    pub collision_group: String,
    /// Then *Physics Impulse*: a random size from this range, along
    /// `impulse_direction` at `impulse_position`, both in the piece's frame.
    pub impulse: [f32; 2],
    pub impulse_position: [f32; 3],
    pub impulse_direction: [f32; 3],
    /// The fade: the material's diffuse colour from the first colour to the
    /// second by the curve; then the pieces are removed.
    pub fade: Progression,
    pub fade_colors: [[f32; 4]; 2],
    pub sounds: PieceSounds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformerType {
    /// Prefab group, e.g. `P_Trafo_Wood`.
    pub group: String,
    /// Ball type name (see [`BallParams::name`]), e.g. `Ball_Wood`.
    pub ball: String,
}

/// One moving element's physics, as its prefab script sets it up. Object and
/// mesh names refer to the prefab scene (`ph/<group>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleSetup {
    /// Prefab group, e.g. `P_Modul_25`.
    pub group: String,
    pub bodies: Vec<ModuleBody>,
    pub hinges: Vec<HingeParams>,
    pub sliders: Vec<SliderParams>,
    pub springs: Vec<SpringParams>,
    pub ball_joints: Vec<BallJointParams>,
    /// The script's `Physics Force` nodes, in script order. When and how long
    /// each acts is the module's script logic.
    pub forces: Vec<ForceParams>,
    /// The script's Delayer times in milliseconds, in script order.
    pub delays_ms: Vec<f32>,
    /// Every `TT Scaleable Proximity` of the script (ball to an object), in
    /// script order (`bodies` empty).
    pub proximities: Vec<WakeParams>,
    /// The objects the script tests the ball's box against (`Box Box
    /// Intersection`'s second entity).
    pub boxes: Vec<String>,
    /// Proximity test that wakes the frozen bodies (`TT Scaleable Proximity`
    /// into `Physics WakeUp`), if the script has one.
    pub wake: Option<WakeParams>,
    /// Physics building blocks the script also uses that aren't in this data
    /// yet (`Set Physics Slider`, `SetPhysicsForce`, …).
    pub unsupported: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the original Physicalize inputs"
)]
pub struct ModuleBody {
    /// The physicalized object.
    pub object: String,
    pub fixed: bool,
    pub friction: f32,
    pub elasticity: f32,
    pub mass: f32,
    pub collision_group: String,
    pub start_frozen: bool,
    pub collision: bool,
    pub auto_mass_center: bool,
    /// Mass centre offset in the object's frame (`Shift Mass Center`).
    pub shift_mass_center: [f32; 3],
    pub linear_damping: f32,
    pub rotational_damping: f32,
    /// Meshes whose convex hulls make up the body, in the object's frame.
    /// Empty: the object's own mesh (the script physicalizes "Get Current").
    pub convex: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HingeParams {
    pub body: String,
    /// The other body; `None` is the world (the level's fixed `FixCube`).
    pub other: Option<String>,
    /// The joint frame: its position is the pivot, its Z axis the hinge axis.
    pub frame: String,
    /// Angle limits in degrees, if enabled.
    pub limits: Option<[f32; 2]>,
    /// The script breaks it (its Destroy is wired to the graph's "Break"
    /// input: the balloon's bridge, at the level's end).
    pub breakable: bool,
}

/// `Set Physics Slider`: `body` moves relative to `other` only along the axis
/// from `axis_from` to `axis_to` (frame positions), without turning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliderParams {
    pub body: String,
    /// `None` is the world.
    pub other: Option<String>,
    pub axis_from: String,
    pub axis_to: String,
    /// Displacement limits along the axis (from where it started), if enabled.
    pub limits: Option<[f32; 2]>,
}

/// `Set Physics Spring` between a point on `body` and a point on `other`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpringParams {
    pub body: String,
    /// `None` is the world.
    pub other: Option<String>,
    /// Where it's attached to `body`: a position in a frame (world if none).
    pub anchor: SpringAnchor,
    /// Where it's attached to `other`.
    pub other_anchor: SpringAnchor,
    pub length: f32,
    pub constant: f32,
    pub linear_damping: f32,
    pub global_damping: f32,
}

/// `Set Physics Ball Joint`: `body` and `other` share a pivot (rotation free).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BallJointParams {
    pub body: String,
    /// `None` is the world.
    pub other: Option<String>,
    /// The pivot: a position in a frame (world if none).
    pub pivot: SpringAnchor,
}

/// `Physics Force`: a per-step push on `body` at a point, along a direction
/// fixed (in world space) when it's created.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForceParams {
    /// `None`: computed by the script (the player's ball).
    pub body: Option<String>,
    /// Where it pushes: a position in a frame (world if none).
    pub position: SpringAnchor,
    /// Its direction in a frame (world if none).
    pub direction: [f32; 3],
    pub direction_frame: Option<String>,
    pub value: f32,
    /// The script graph the force is in (e.g. the balloon's `Balloon Forces`).
    pub graph: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpringAnchor {
    pub frame: Option<String>,
    pub position: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WakeParams {
    /// The object the ball's distance is measured to.
    pub object: String,
    pub distance: f32,
    pub exactness: [f32; 2],
    pub frame_delay: [i32; 2],
    /// Only X and Z count.
    pub xz: bool,
    /// The bodies `Physics WakeUp` is applied to.
    pub bodies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BallParams {
    /// Ball object name, e.g. `Ball_Wood`.
    pub name: String,
    pub friction: f32,
    pub elasticity: f32,
    pub mass: f32,
    pub linear_damping: f32,
    pub rotational_damping: f32,
    /// Control push per physics step (see the physics docs).
    pub force: f32,
    pub collision_group: String,
    pub shape: BallShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BallShape {
    /// Exact sphere primitive.
    Sphere { radius: f32 },
    /// Convex hull of the ball's mesh; `radius` is its bounding radius.
    Convex { radius: f32 },
}

impl BallShape {
    pub fn radius(self) -> f32 {
        match self {
            Self::Sphere { radius } | Self::Convex { radius } => radius,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceParams {
    /// Level group, e.g. `Phys_Floors`.
    pub group: String,
    pub friction: f32,
    pub elasticity: f32,
    pub collision_group: String,
    pub collision: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the original Physicalize table columns"
)]
pub struct PropParams {
    /// Prefab group, e.g. `P_Box`.
    pub group: String,
    pub fixed: bool,
    pub friction: f32,
    pub elasticity: f32,
    pub mass: f32,
    pub collision_group: String,
    pub start_frozen: bool,
    pub collision: bool,
    pub auto_mass_center: bool,
    pub linear_damping: f32,
    pub rotational_damping: f32,
    /// Set for props physicalized as a sphere primitive.
    pub sphere_radius: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_hermite_curves() {
        // AnimTrafo's Ring_Close: (0, 1) to (1, 0), flat start, steep end.
        let close = [
            CurvePoint {
                position: [0.0, 1.0],
                in_tangent: [0.0001, 0.000_005],
                out_tangent: [1.0, 0.05],
            },
            CurvePoint {
                position: [1.0, 0.0],
                in_tangent: [1.0, -2.48],
                out_tangent: [0.001, -0.002_48],
            },
        ];
        assert!((curve_y(&close, 0.0) - 1.0).abs() < 1e-6);
        assert!((curve_y(&close, 1.0)).abs() < 1e-6);
        // x is linear in the parameter: t = 0.5 at x = 0.5.
        let t = 0.5_f32;
        let expected = (2.0 * t.powi(3) - 3.0 * t * t + 1.0) * 1.0
            + (t.powi(3) - 2.0 * t * t + t) * 0.05
            + (t.powi(3) - t * t) * -2.48;
        assert!((curve_y(&close, 0.5) - expected).abs() < 1e-4);
        // Outside: the end values.
        assert!((curve_y(&close, -1.0) - 1.0).abs() < 1e-6);
        let p = Progression {
            duration_ms: 200.0,
            curve: close.to_vec(),
        };
        assert!((p.value(100.0) - expected).abs() < 1e-4);
        assert!(p.value(500.0).abs() < 1e-6 && p.done(200.0));
    }
}
