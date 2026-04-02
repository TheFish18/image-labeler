#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rectangle { min: Point, max: Point },
    Polygon { points: Vec<Point> },
}

impl Shape {
    pub fn normalized(self) -> Option<Self> {
        match self {
            Self::Rectangle { min, max } => {
                let left = min.x.min(max.x);
                let right = min.x.max(max.x);
                let top = min.y.min(max.y);
                let bottom = min.y.max(max.y);
                if (right - left) < 1.0 || (bottom - top) < 1.0 {
                    None
                } else {
                    Some(Self::Rectangle {
                        min: Point::new(left, top),
                        max: Point::new(right, bottom),
                    })
                }
            }
            Self::Polygon { points } => {
                if points.len() < 3 {
                    None
                } else {
                    Some(Self::Polygon { points })
                }
            }
        }
    }

    pub fn points(&self) -> Vec<Point> {
        match self {
            Self::Rectangle { min, max } => vec![
                *min,
                Point::new(max.x, min.y),
                *max,
                Point::new(min.x, max.y),
            ],
            Self::Polygon { points } => points.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Annotation {
    pub id: i64,
    pub class_name: String,
    pub color_rgb: [u8; 3],
    pub shape: Shape,
}
