use ckfile::{ClassId, FileObject, Result};

use crate::{Entity2d, Entity3d, Group, Material, Mesh, Texture};

/// A decoded object of one of the supported classes.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Decoded {
    Mesh(Mesh),
    Material(Material),
    Texture(Texture),
    Entity(Entity3d),
    Entity2d(Entity2d),
    Group(Group),
}

/// Decodes `obj` if its class is supported. Returns `None` for other classes
/// and for objects saved without data.
pub fn decode_object(obj: &FileObject) -> Option<Result<Decoded>> {
    let chunk = obj.chunk.as_ref()?;
    Some(match obj.class_id {
        ClassId::MESH => Mesh::decode(chunk).map(Decoded::Mesh),
        ClassId::MATERIAL => Material::decode(chunk).map(Decoded::Material),
        ClassId::TEXTURE => Texture::decode(chunk).map(Decoded::Texture),
        ClassId::GROUP => Group::decode(chunk).map(Decoded::Group),
        ClassId::ENTITY_2D => Entity2d::decode(chunk).map(Decoded::Entity2d),
        ClassId::ENTITY_3D
        | ClassId::OBJECT_3D
        | ClassId::CAMERA
        | ClassId::TARGET_CAMERA
        | ClassId::LIGHT
        | ClassId::TARGET_LIGHT
        | ClassId::SPRITE_3D => Entity3d::decode(chunk).map(Decoded::Entity),
        _ => return None,
    })
}
