//! Bounding-volume hierarchy over static triangles (built once per level).

use glam::Vec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb {
        min: Vec3::splat(f32::MAX),
        max: Vec3::splat(f32::MIN),
    };

    pub fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(o.min),
            max: self.max.max(o.max),
        }
    }

    pub fn overlaps(&self, o: &Aabb) -> bool {
        self.min.cmple(o.max).all() && o.min.cmple(self.max).all()
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
}

#[derive(Debug, Clone)]
enum Node {
    Leaf {
        bounds: Aabb,
        first: u32,
        count: u32,
    },
    Inner {
        bounds: Aabb,
        left: u32,
        right: u32,
    },
}

impl Node {
    fn bounds(&self) -> &Aabb {
        match self {
            Node::Leaf { bounds, .. } | Node::Inner { bounds, .. } => bounds,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    /// Item indices, grouped by leaf.
    items: Vec<u32>,
}

const LEAF_SIZE: usize = 4;

impl Bvh {
    pub fn build(boxes: &[Aabb]) -> Self {
        let mut bvh = Bvh {
            nodes: Vec::new(),
            items: (0..boxes.len() as u32).collect(),
        };
        if !boxes.is_empty() {
            bvh.build_node(boxes, 0, boxes.len());
        }
        bvh
    }

    fn build_node(&mut self, boxes: &[Aabb], start: usize, end: usize) -> u32 {
        let bounds = self.items[start..end]
            .iter()
            .fold(Aabb::EMPTY, |a, &i| a.union(&boxes[i as usize]));
        let index = self.nodes.len() as u32;
        if end - start <= LEAF_SIZE {
            self.nodes.push(Node::Leaf {
                bounds,
                first: start as u32,
                count: (end - start) as u32,
            });
            return index;
        }
        // Split at the median along the longest axis of the centroid bounds.
        let mut cb = Aabb::EMPTY;
        for &i in &self.items[start..end] {
            cb.grow(boxes[i as usize].center());
        }
        let ext = cb.max - cb.min;
        let axis = if ext.x >= ext.y && ext.x >= ext.z {
            0
        } else if ext.y >= ext.z {
            1
        } else {
            2
        };
        // Stable sort keeps the build deterministic.
        self.items[start..end].sort_by(|&a, &b| {
            boxes[a as usize].center()[axis].total_cmp(&boxes[b as usize].center()[axis])
        });
        let mid = start + (end - start) / 2;
        self.nodes.push(Node::Leaf {
            bounds,
            first: 0,
            count: 0,
        });
        let left = self.build_node(boxes, start, mid);
        let right = self.build_node(boxes, mid, end);
        self.nodes[index as usize] = Node::Inner {
            bounds,
            left,
            right,
        };
        index
    }

    /// Calls `f` with every item whose leaf overlaps `query`, in a fixed order.
    pub fn query(&self, query: &Aabb, mut f: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            if !node.bounds().overlaps(query) {
                continue;
            }
            match node {
                Node::Leaf { first, count, .. } => {
                    for &i in &self.items[*first as usize..(*first + *count) as usize] {
                        f(i);
                    }
                }
                Node::Inner { left, right, .. } => {
                    stack.push(*right);
                    stack.push(*left);
                }
            }
        }
    }
}
