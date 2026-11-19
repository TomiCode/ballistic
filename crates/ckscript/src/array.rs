//! `DataArray` (class 52): typed tables the scripts read and write at runtime
//! (physics settings per ball type, per-level data, menu layouts…).

use ckfile::{Guid, Result, StateChunk};

use crate::{ObjRef, bad, missing, obj_ref};

const SECTION_COLUMNS: u32 = 0x1000;
const SECTION_ROWS: u32 = 0x2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ColumnType {
    Int,
    Float,
    String,
    Object,
    /// Each cell references a `Parameter` object of this type.
    Parameter(Guid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Column {
    pub name: String,
    pub ty: ColumnType,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Cell {
    Int(i32),
    Float(f32),
    String(String),
    Object(Option<ObjRef>),
    /// Reference to the `Parameter` object holding the value.
    Parameter(Option<ObjRef>),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataArray {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
}

impl DataArray {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_COLUMNS)?
            .ok_or_else(|| missing("DataArray", SECTION_COLUMNS))?;
        let n = r.u32()? as usize;
        let mut columns = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.string()?.unwrap_or_default();
            let ty = match r.u32()? {
                1 => ColumnType::Int,
                2 => ColumnType::Float,
                3 => ColumnType::String,
                4 => ColumnType::Object,
                5 => ColumnType::Parameter(r.guid()?),
                t => return Err(bad(format!("DataArray: unknown column type {t}"))),
            };
            columns.push(Column { name, ty });
        }
        if !r.is_empty() {
            return Err(bad("DataArray: unread words in column section"));
        }

        let mut rows = Vec::new();
        if let Some(mut r) = chunk.section(SECTION_ROWS)? {
            let n_rows = r.u32()? as usize;
            for _ in 0..n_rows {
                let mut row = Vec::with_capacity(columns.len());
                for c in &columns {
                    row.push(match c.ty {
                        ColumnType::Int => Cell::Int(r.i32()?),
                        ColumnType::Float => Cell::Float(r.f32()?),
                        ColumnType::String => Cell::String(r.string()?.unwrap_or_default()),
                        ColumnType::Object => Cell::Object(obj_ref(r.u32()?)),
                        ColumnType::Parameter(_) => Cell::Parameter(obj_ref(r.u32()?)),
                    });
                }
                rows.push(row);
            }
            if !r.is_empty() {
                return Err(bad(format!(
                    "DataArray: {} unread words in row section",
                    r.remaining()
                )));
            }
        }
        Ok(Self { columns, rows })
    }

    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// Row whose first column is the string `key`.
    pub fn row_by_key(&self, key: &str) -> Option<&[Cell]> {
        self.rows
            .iter()
            .find(|r| matches!(r.first(), Some(Cell::String(s)) if s == key))
            .map(Vec::as_slice)
    }
}
