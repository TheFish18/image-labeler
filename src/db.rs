use crate::geometry::{Annotation, Point, Shape};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database {}", path.display()))?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS images (
                hash TEXT PRIMARY KEY,
                width INTEGER NOT NULL,
                height INTEGER NOT NULL,
                bit_depth INTEGER NOT NULL,
                last_path TEXT
            );

            CREATE TABLE IF NOT EXISTS classes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                color_r INTEGER NOT NULL,
                color_g INTEGER NOT NULL,
                color_b INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS annotations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                image_hash TEXT NOT NULL,
                class_id INTEGER NOT NULL,
                shape_type TEXT NOT NULL,
                x REAL,
                y REAL,
                width REAL,
                height REAL,
                FOREIGN KEY(image_hash) REFERENCES images(hash) ON DELETE CASCADE,
                FOREIGN KEY(class_id) REFERENCES classes(id) ON DELETE RESTRICT
            );

            CREATE TABLE IF NOT EXISTS annotation_points (
                annotation_id INTEGER NOT NULL,
                point_index INTEGER NOT NULL,
                x REAL NOT NULL,
                y REAL NOT NULL,
                PRIMARY KEY(annotation_id, point_index),
                FOREIGN KEY(annotation_id) REFERENCES annotations(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS image_classifications (
                image_hash TEXT NOT NULL,
                class_id INTEGER NOT NULL,
                PRIMARY KEY(image_hash, class_id),
                FOREIGN KEY(image_hash) REFERENCES images(hash) ON DELETE CASCADE,
                FOREIGN KEY(class_id) REFERENCES classes(id) ON DELETE CASCADE
            );
            ",
        )?;

        Ok(())
    }

    pub fn upsert_image(
        &self,
        hash: &str,
        width: usize,
        height: usize,
        bit_depth: u8,
        path: &str,
    ) -> Result<()> {
        self.conn.execute(
            "
            INSERT INTO images(hash, width, height, bit_depth, last_path)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(hash) DO UPDATE SET
                width = excluded.width,
                height = excluded.height,
                bit_depth = excluded.bit_depth,
                last_path = excluded.last_path
            ",
            params![hash, width as i64, height as i64, bit_depth as i64, path],
        )?;
        Ok(())
    }

    pub fn upsert_class(&self, name: &str, color_rgb: [u8; 3]) -> Result<i64> {
        self.conn.execute(
            "
            INSERT INTO classes(name, color_r, color_g, color_b)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(name) DO UPDATE SET
                color_r = excluded.color_r,
                color_g = excluded.color_g,
                color_b = excluded.color_b
            ",
            params![name, color_rgb[0], color_rgb[1], color_rgb[2]],
        )?;

        self.conn
            .query_row(
                "SELECT id FROM classes WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .context("failed to fetch class id after upsert")
    }

    pub fn list_image_classifications(&self, image_hash: &str) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT class_id
            FROM image_classifications
            WHERE image_hash = ?1
            ORDER BY class_id ASC
            ",
        )?;
        let rows = stmt.query_map(params![image_hash], |row| row.get(0))?;
        let mut class_ids = Vec::new();
        for row in rows {
            class_ids.push(row?);
        }
        Ok(class_ids)
    }

    pub fn set_image_classification(
        &self,
        image_hash: &str,
        class_id: i64,
        present: bool,
    ) -> Result<()> {
        if present {
            self.conn.execute(
                "
                INSERT INTO image_classifications(image_hash, class_id)
                VALUES (?1, ?2)
                ON CONFLICT(image_hash, class_id) DO NOTHING
                ",
                params![image_hash, class_id],
            )?;
        } else {
            self.conn.execute(
                "
                DELETE FROM image_classifications
                WHERE image_hash = ?1 AND class_id = ?2
                ",
                params![image_hash, class_id],
            )?;
        }
        Ok(())
    }

    pub fn list_annotations(&self, image_hash: &str) -> Result<Vec<Annotation>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT a.id, a.class_id, c.name, c.color_r, c.color_g, c.color_b,
                   a.shape_type, a.x, a.y, a.width, a.height
            FROM annotations a
            JOIN classes c ON c.id = a.class_id
            WHERE a.image_hash = ?1
            ORDER BY a.id ASC
            ",
        )?;

        let base_rows = stmt.query_map(params![image_hash], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                [
                    row.get::<_, u8>(3)?,
                    row.get::<_, u8>(4)?,
                    row.get::<_, u8>(5)?,
                ],
                row.get::<_, String>(6)?,
                row.get::<_, Option<f32>>(7)?,
                row.get::<_, Option<f32>>(8)?,
                row.get::<_, Option<f32>>(9)?,
                row.get::<_, Option<f32>>(10)?,
            ))
        })?;

        let mut annotations = Vec::new();
        for row in base_rows {
            let (id, _class_id, class_name, color_rgb, shape_type, x, y, width, height) = row?;
            let shape = if shape_type == "rectangle" {
                Shape::Rectangle {
                    min: Point::new(x.unwrap_or_default(), y.unwrap_or_default()),
                    max: Point::new(
                        x.unwrap_or_default() + width.unwrap_or_default(),
                        y.unwrap_or_default() + height.unwrap_or_default(),
                    ),
                }
            } else {
                Shape::Polygon {
                    points: self.load_polygon_points(id)?,
                }
            };
            annotations.push(Annotation {
                id,
                class_name,
                color_rgb,
                shape,
            });
        }
        Ok(annotations)
    }

    fn load_polygon_points(&self, annotation_id: i64) -> Result<Vec<Point>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT x, y
            FROM annotation_points
            WHERE annotation_id = ?1
            ORDER BY point_index ASC
            ",
        )?;
        let rows = stmt.query_map(params![annotation_id], |row| {
            Ok(Point::new(row.get(0)?, row.get(1)?))
        })?;
        let mut points = Vec::new();
        for row in rows {
            points.push(row?);
        }
        Ok(points)
    }

    pub fn insert_annotation(&self, image_hash: &str, class_id: i64, shape: &Shape) -> Result<i64> {
        match shape {
            Shape::Rectangle { min, max } => {
                self.conn.execute(
                    "
                    INSERT INTO annotations(image_hash, class_id, shape_type, x, y, width, height)
                    VALUES (?1, ?2, 'rectangle', ?3, ?4, ?5, ?6)
                    ",
                    params![
                        image_hash,
                        class_id,
                        min.x,
                        min.y,
                        max.x - min.x,
                        max.y - min.y
                    ],
                )?;
            }
            Shape::Polygon { points } => {
                self.conn.execute(
                    "
                    INSERT INTO annotations(image_hash, class_id, shape_type)
                    VALUES (?1, ?2, 'polygon')
                    ",
                    params![image_hash, class_id],
                )?;
                let annotation_id = self.conn.last_insert_rowid();
                for (index, point) in points.iter().enumerate() {
                    self.conn.execute(
                        "
                        INSERT INTO annotation_points(annotation_id, point_index, x, y)
                        VALUES (?1, ?2, ?3, ?4)
                        ",
                        params![annotation_id, index as i64, point.x, point.y],
                    )?;
                }
                return Ok(annotation_id);
            }
        }

        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_annotation(&self, annotation_id: i64, shape: &Shape) -> Result<()> {
        match shape {
            Shape::Rectangle { min, max } => {
                self.conn.execute(
                    "
                    UPDATE annotations
                    SET shape_type = 'rectangle', x = ?2, y = ?3, width = ?4, height = ?5
                    WHERE id = ?1
                    ",
                    params![annotation_id, min.x, min.y, max.x - min.x, max.y - min.y],
                )?;
                self.conn.execute(
                    "DELETE FROM annotation_points WHERE annotation_id = ?1",
                    params![annotation_id],
                )?;
            }
            Shape::Polygon { points } => {
                self.conn.execute(
                    "
                    UPDATE annotations
                    SET shape_type = 'polygon', x = NULL, y = NULL, width = NULL, height = NULL
                    WHERE id = ?1
                    ",
                    params![annotation_id],
                )?;
                self.conn.execute(
                    "DELETE FROM annotation_points WHERE annotation_id = ?1",
                    params![annotation_id],
                )?;
                for (index, point) in points.iter().enumerate() {
                    self.conn.execute(
                        "
                        INSERT INTO annotation_points(annotation_id, point_index, x, y)
                        VALUES (?1, ?2, ?3, ?4)
                        ",
                        params![annotation_id, index as i64, point.x, point.y],
                    )?;
                }
            }
        }
        Ok(())
    }

    pub fn delete_annotation(&self, annotation_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM annotations WHERE id = ?1",
            params![annotation_id],
        )?;
        Ok(())
    }
}
