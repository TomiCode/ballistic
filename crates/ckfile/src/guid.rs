use std::fmt;

/// A Virtools GUID: two 32-bit words. Identifies classes of plugins, managers and
/// building blocks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Guid(pub u32, pub u32);

impl Guid {
    pub const NULL: Guid = Guid(0, 0);
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08x}-{:08x}", self.0, self.1)
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Guid({self})")
    }
}
