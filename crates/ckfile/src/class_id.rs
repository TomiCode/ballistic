use std::fmt;

/// Virtools class identifier (`CKCID`). Stored as a raw value, so unknown IDs
/// survive a round trip.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ClassId(pub u32);

macro_rules! class_ids {
    ($($name:ident = $value:literal => $display:literal,)*) => {
        impl ClassId {
            $(pub const $name: ClassId = ClassId($value);)*

            /// Human-readable class name, if known.
            pub fn name(self) -> Option<&'static str> {
                match self.0 {
                    $($value => Some($display),)*
                    _ => None,
                }
            }
        }
    };
}

class_ids! {
    OBJECT = 1 => "Object",
    PARAMETER_IN = 2 => "ParameterIn",
    PARAMETER_OUT = 3 => "ParameterOut",
    PARAMETER_OPERATION = 4 => "ParameterOperation",
    STATE = 5 => "State",
    BEHAVIOR_LINK = 6 => "BehaviorLink",
    BEHAVIOR = 8 => "Behavior",
    BEHAVIOR_IO = 9 => "BehaviorIO",
    SCENE = 10 => "Scene",
    SCENE_OBJECT = 11 => "SceneObject",
    RENDER_CONTEXT = 12 => "RenderContext",
    KINEMATIC_CHAIN = 13 => "KinematicChain",
    OBJECT_ANIMATION = 15 => "ObjectAnimation",
    ANIMATION = 16 => "Animation",
    KEYED_ANIMATION = 18 => "KeyedAnimation",
    BE_OBJECT = 19 => "BeObject",
    LEVEL = 21 => "Level",
    PLACE = 22 => "Place",
    GROUP = 23 => "Group",
    SOUND = 24 => "Sound",
    WAVE_SOUND = 25 => "WaveSound",
    MIDI_SOUND = 26 => "MidiSound",
    ENTITY_2D = 27 => "2dEntity",
    SPRITE = 28 => "Sprite",
    SPRITE_TEXT = 29 => "SpriteText",
    MATERIAL = 30 => "Material",
    TEXTURE = 31 => "Texture",
    MESH = 32 => "Mesh",
    ENTITY_3D = 33 => "3dEntity",
    CAMERA = 34 => "Camera",
    TARGET_CAMERA = 35 => "TargetCamera",
    CURVE_POINT = 36 => "CurvePoint",
    SPRITE_3D = 37 => "Sprite3D",
    LIGHT = 38 => "Light",
    TARGET_LIGHT = 39 => "TargetLight",
    CHARACTER = 40 => "Character",
    OBJECT_3D = 41 => "3dObject",
    BODY_PART = 42 => "BodyPart",
    CURVE = 43 => "Curve",
    PARAMETER_LOCAL = 45 => "ParameterLocal",
    PARAMETER = 46 => "Parameter",
    RENDER_OBJECT = 47 => "RenderObject",
    INTERFACE_OBJECT_MANAGER = 48 => "InterfaceObjectManager",
    CRITICAL_SECTION = 49 => "CriticalSection",
    GRID = 50 => "Grid",
    LAYER = 51 => "Layer",
    DATA_ARRAY = 52 => "DataArray",
    PATCH_MESH = 53 => "PatchMesh",
    PROGRESSIVE_MESH = 54 => "ProgressiveMesh",
    SYNCHRO = 55 => "Synchro",
    STATE_OBJECT = 56 => "StateObject",
    VIDEO = 57 => "Video",
    OBJECT_ARRAY = 80 => "ObjectArray",
}

impl fmt::Display for ClassId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "Class#{}", self.0),
        }
    }
}

impl fmt::Debug for ClassId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ClassId({}={self})", self.0)
    }
}
