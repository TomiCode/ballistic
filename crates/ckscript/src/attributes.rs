//! Object attributes (research findings 15).
//!
//! The attribute manager's chunk (`3d242466-…`) lists the file's attribute
//! types:
//! - two words, the number of categories, the number of attributes;
//! - each category slot: `0` when unused, else `1`, the name's byte length,
//!   the name (padded to 4), one flags word;
//! - each attribute slot: `0` when unused, else `1`, the name's byte length,
//!   the name, the parameter type GUID (2 words), the category index and two
//!   flags words.
//!
//! An object's attributes are in its chunk's section `0x11`: an object list
//! (count, then the value parameters, `ParameterOut` objects) and a manager
//! list (count, the attribute manager's GUID, then each attribute's index).

use ckfile::{CkFile, Guid};

use crate::extract::{Value, typed_value};
use crate::{ScriptObject, Scripts};

/// The attribute manager.
pub const ATTRIBUTE_MANAGER: Guid = Guid(0x3d24_2466, 0);

/// Section of an object's chunk that holds its attributes.
const SECTION_ATTRIBUTES: u32 = 0x11;

/// The file's attribute type names, by index (`None` for unused slots).
pub fn attribute_names(file: &CkFile) -> Vec<Option<String>> {
    let Some(w) = file
        .managers
        .iter()
        .find(|m| m.guid == ATTRIBUTE_MANAGER)
        .and_then(|m| m.chunk.as_ref())
        .map(|c| c.data.as_slice())
    else {
        return Vec::new();
    };
    let get = |i: usize| w.get(i).copied();
    let name_at = |i: usize| -> Option<(String, usize)> {
        let len = get(i + 1)? as usize;
        let n = len.div_ceil(4);
        let bytes: Vec<u8> = w
            .get(i + 2..i + 2 + n)?
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .take(len)
            .collect();
        let name = ckfile::decode_cp1252(&bytes);
        Some((name.trim_end_matches('\0').to_owned(), 2 + n))
    };
    let (Some(categories), Some(count)) = (get(2), get(3)) else {
        return Vec::new();
    };
    let mut i = 4;
    for _ in 0..categories {
        match get(i) {
            Some(0) => i += 1,
            Some(_) => match name_at(i) {
                Some((_, words)) => i += words + 1,
                None => return Vec::new(),
            },
            None => return Vec::new(),
        }
    }
    let mut out = Vec::new();
    for _ in 0..count {
        match get(i) {
            Some(0) => {
                out.push(None);
                i += 1;
            }
            Some(_) => match name_at(i) {
                Some((name, words)) => {
                    out.push(Some(name));
                    i += words + 5;
                }
                None => break,
            },
            None => break,
        }
    }
    out
}

/// An object's attributes: (name, value). `names` is [`attribute_names`] of
/// the same file.
pub fn object_attributes(
    file: &CkFile,
    scripts: &Scripts,
    names: &[Option<String>],
    object: usize,
) -> Vec<(String, Value)> {
    let Some(mut r) = file
        .objects
        .get(object)
        .and_then(|o| o.chunk.as_ref())
        .and_then(|c| c.section(SECTION_ATTRIBUTES).ok().flatten())
    else {
        return Vec::new();
    };
    let read = |r: &mut ckfile::ChunkReader<'_>| -> Option<Vec<(usize, usize)>> {
        let n = r.u32().ok()? as usize;
        let params: Vec<usize> = r.dwords(n).ok()?.iter().map(|&x| x as usize).collect();
        let m = r.u32().ok()? as usize;
        let guid = Guid(r.u32().ok()?, r.u32().ok()?);
        if guid != ATTRIBUTE_MANAGER || m != n {
            return None;
        }
        let idx: Vec<usize> = r.dwords(m).ok()?.iter().map(|&x| x as usize).collect();
        Some(params.into_iter().zip(idx).collect())
    };
    let Some(pairs) = read(&mut r) else {
        return Vec::new();
    };
    pairs
        .into_iter()
        .filter_map(|(param, idx)| {
            let name = names.get(idx)?.clone()?;
            let value = match scripts.objects.get(param) {
                Some(Some(ScriptObject::Out(p))) => typed_value(file, p.type_guid, &p.value),
                _ => Value::Object(None),
            };
            Some((name, value))
        })
        .collect()
}
