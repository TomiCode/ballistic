//! Builds static collision geometry from a bundle level scene.

use ballistic_bundle as bb;
use glam::{Mat4, Vec3};

use crate::mesh::{StaticMesh, SurfaceMaterial};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelBuildReport {
    /// (group, objects, triangles)
    pub groups: Vec<(String, usize, usize)>,
}

/// Collects the triangles of every object in the level groups named by
/// `physics.surfaces` (`Phys_Floors`, `Phys_FloorRails`, `Phys_FloorStopper`).
pub fn build_level(scene: &bb::Scene, physics: &bb::PhysicsData) -> (StaticMesh, LevelBuildReport) {
    let mut mesh = StaticMesh::default();
    let mut report = LevelBuildReport::default();
    for s in &physics.surfaces {
        if !s.collision {
            continue;
        }
        let Some(group) = scene.groups.iter().find(|g| g.name == s.group) else {
            continue;
        };
        let material = mesh.add_material(SurfaceMaterial {
            friction: s.friction,
            elasticity: s.elasticity,
            group: s.collision_group.clone(),
        });
        let before = mesh.triangle_count();
        for &oi in &group.members {
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
