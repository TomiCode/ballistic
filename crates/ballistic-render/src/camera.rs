use glam::{Mat4, Vec3};

/// Free-fly camera. Right-handed, Y up; `yaw = 0` looks down -Z.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    /// Radians, positive turns right.
    pub yaw: f32,
    /// Radians, positive looks up.
    pub pitch: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 60f32.to_radians(),
            near: 0.5,
            far: 5000.0,
        }
    }
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }

    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, sy)
    }

    /// Points the camera at `target`.
    pub fn look_at(&mut self, target: Vec3) {
        let d = (target - self.position).normalize_or(Vec3::NEG_Z);
        self.pitch = d.y.clamp(-1.0, 1.0).asin();
        self.yaw = d.x.atan2(-d.z);
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_to_mat4(self.position, self.forward(), Vec3::Y)
    }

    /// Projection with wgpu's clip conventions (Y up, depth 0..1).
    pub fn projection(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective(self.fov_y, aspect, self.near, self.far)
    }

    /// Moves in camera space: x = right, y = world up, z = forward.
    pub fn fly(&mut self, local: Vec3) {
        self.position += self.right() * local.x + Vec3::Y * local.y + self.forward() * local.z;
    }

    pub fn rotate(&mut self, d_yaw: f32, d_pitch: f32) {
        self.yaw = (self.yaw + d_yaw).rem_euclid(std::f32::consts::TAU);
        let limit = 89f32.to_radians();
        self.pitch = (self.pitch + d_pitch).clamp(-limit, limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_at_points_forward_at_target() {
        let mut c = Camera {
            position: Vec3::new(1.0, 5.0, 2.0),
            ..Default::default()
        };
        let target = Vec3::new(-4.0, 0.0, 9.0);
        c.look_at(target);
        let expected = (target - c.position).normalize();
        assert!(c.forward().abs_diff_eq(expected, 1e-5));
        // Forward maps to -Z in view space.
        let v = c.view().transform_vector3(c.forward());
        assert!(v.abs_diff_eq(Vec3::NEG_Z, 1e-5));
    }

    #[test]
    fn right_is_perpendicular_and_horizontal() {
        let c = Camera {
            yaw: 0.7,
            pitch: 0.3,
            ..Default::default()
        };
        assert!(c.right().dot(c.forward()).abs() < 1e-5);
        assert!(c.right().y.abs() < 1e-6);
        assert!(c.right().cross(c.forward()).y > 0.0);
    }
}
