//! Builds static collision geometry from a bundle level scene.

use ballistic_bundle as bb;
use glam::{Mat4, Vec3};

use std::collections::HashMap;

use crate::mesh::{SoundIds, StaticMesh, SurfaceMaterial};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelBuildReport {
    /// (group, objects, triangles)
    pub groups: Vec<(String, usize, usize)>,
}

/// Collects the triangles of every object in the level groups named by
/// `physics.surfaces` (`Phys_Floors`, `Phys_FloorRails`, `Phys_FloorStopper`).
/// Each object's sound IDs come from the level's sound groups
/// (`Sound_HitID_NN`, `Sound_RollID_NN`), as level init assigns them.
pub fn build_level(scene: &bb::Scene, physics: &bb::PhysicsData) -> (StaticMesh, LevelBuildReport) {
    let mut mesh = StaticMesh::default();
    let mut report = LevelBuildReport::default();
    let sound = level_sounds(scene, &physics.sounds);
    for s in &physics.surfaces {
        if !s.collision {
            continue;
        }
        let Some(group) = scene.groups.iter().find(|g| g.name == s.group) else {
            continue;
        };
        let mut materials: HashMap<SoundIds, u32> = HashMap::new();
        let before = mesh.triangle_count();
        for &oi in &group.members {
            let ids = sound.get(&(oi as usize)).copied().unwrap_or_default();
            let material = *materials.entry(ids).or_insert_with(|| {
                mesh.add_material(SurfaceMaterial {
                    friction: s.friction,
                    elasticity: s.elasticity,
                    group: s.collision_group.clone(),
                    sound: ids,
                })
            });
            let obj = &scene.objects[oi as usize];
            let Some(mi) = obj.mesh else { continue };
            let m = &scene.meshes[mi as usize];
            let xf = Mat4::from_cols_array_2d(&obj.transform);
            let verts: Vec<Vec3> = m
                .vertices()
                .iter()
                .map(|v| xf.transform_point3(Vec3::from(v.position)))
                .collect();
            for tri in m.indices().as_chunks::<3>().0 {
                let [a, b, c] = tri.map(|i| verts[usize::from(i)]);
                mesh.add_triangle(a, b, c, material);
            }
        }
        report.groups.push((
            s.group.clone(),
            group.members.len(),
            mesh.triangle_count() - before,
        ));
    }
    mesh.finish();
    (mesh, report)
}

/// Level objects' sound IDs from the level's sound groups.
pub fn level_sounds(scene: &bb::Scene, sounds: &bb::BallSounds) -> HashMap<usize, SoundIds> {
    let mut out: HashMap<usize, SoundIds> = HashMap::new();
    let groups = |list: &[bb::SoundGroup]| -> Vec<(Vec<u32>, u8)> {
        list.iter()
            .filter_map(|sg| {
                let g = scene.groups.iter().find(|g| g.name == sg.group)?;
                Some((g.members.clone(), sg.id))
            })
            .collect()
    };
    for (members, id) in groups(&sounds.hit_groups) {
        for m in members {
            out.entry(m as usize).or_default().hit = id;
        }
    }
    for (members, id) in groups(&sounds.roll_groups) {
        for m in members {
            out.entry(m as usize).or_default().roll = id;
        }
    }
    out
}
