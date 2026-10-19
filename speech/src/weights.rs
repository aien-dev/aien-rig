//! Minimal safetensors reader (memory-mapped) and Whisper tokenizer decoding.
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use memmap2::Mmap;
use serde_json::Value;

pub struct SafeTensors {
    map: Mmap,
    data_start: usize,
    tensors: HashMap<String, (String, Vec<usize>, usize, usize)>,
}

impl SafeTensors {
    pub fn open(path: &Path) -> Result<Self, String> {
        let f = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        // SAFETY: read-only map of a file we do not modify while it is open.
        let map = unsafe { Mmap::map(&f) }.map_err(|e| format!("mmap {}: {e}", path.display()))?;
        if map.len() < 8 {
            return Err("safetensors file too short".into());
        }
        let hlen = u64::from_le_bytes(map[0..8].try_into().unwrap()) as usize;
        let header: Value = serde_json::from_slice(&map[8..8 + hlen]).map_err(|e| format!("safetensors header: {e}"))?;
        let mut tensors = HashMap::new();
        for (name, v) in header.as_object().ok_or("header is not an object")? {
            if name == "__metadata__" {
                continue;
            }
            let dtype = v["dtype"].as_str().ok_or("dtype")?.to_string();
            let shape: Vec<usize> = v["shape"].as_array().ok_or("shape")?.iter().map(|x| x.as_u64().unwrap() as usize).collect();
            let off = v["data_offsets"].as_array().ok_or("data_offsets")?;
            let (a, b) = (off[0].as_u64().unwrap() as usize, off[1].as_u64().unwrap() as usize);
            tensors.insert(name.clone(), (dtype, shape, a, b));
        }
        Ok(Self { map, data_start: 8 + hlen, tensors })
    }

    pub fn shape(&self, name: &str) -> Option<&[usize]> {
        self.tensors.get(name).map(|t| t.1.as_slice())
    }

    /// Tensor as f32 (F16, BF16 and F32 supported).
    pub fn f32(&self, name: &str) -> Result<(Vec<f32>, Vec<usize>), String> {
        let (dtype, shape, a, b) = self.tensors.get(name).ok_or_else(|| format!("tensor {name} missing"))?;
        let bytes = &self.map[self.data_start + a..self.data_start + b];
        let n: usize = shape.iter().product();
        let v: Vec<f32> = match dtype.as_str() {
            "F16" => bytes.chunks_exact(2).map(|c| half::f16::from_le_bytes([c[0], c[1]]).to_f32()).collect(),
            "BF16" => bytes.chunks_exact(2).map(|c| half::bf16::from_le_bytes([c[0], c[1]]).to_f32()).collect(),
            "F32" => bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
            d => return Err(format!("{name}: dtype {d} not supported")),
        };
        if v.len() != n {
            return Err(format!("{name}: {} values for shape {shape:?}", v.len()));
        }
        Ok((v, shape.clone()))
    }
}

/// Token ids to text for Whisper's byte-level BPE (tokenizer.json from Hugging Face).
pub struct Tokenizer {
    pieces: Vec<Option<Vec<u8>>>, // id -> raw bytes (None for specials)
    pub specials: HashMap<String, u32>,
    pub special_by_id: HashMap<u32, String>,
}

/// GPT-2's bytes_to_unicode table, inverted: char -> byte.
fn unicode_to_byte() -> HashMap<char, u8> {
    let mut bs: Vec<u32> = Vec::new();
    bs.extend(b'!' as u32..=b'~' as u32);
    bs.extend(0xA1u32..=0xAC);
    bs.extend(0xAEu32..=0xFF);
    let mut cs = bs.clone();
    let mut n = 0;
    for b in 0..256u32 {
        if !bs.contains(&b) {
            bs.push(b);
            cs.push(256 + n);
            n += 1;
        }
    }
    bs.iter().zip(cs.iter()).map(|(&b, &c)| (char::from_u32(c).unwrap(), b as u8)).collect()
}

impl Tokenizer {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("tokenizer.json: {e}"))?;
        let u2b = unicode_to_byte();
        let vocab = v["model"]["vocab"].as_object().ok_or("tokenizer.json: model.vocab missing")?;
        let mut max_id = 0usize;
        let mut entries = Vec::with_capacity(vocab.len());
        for (s, id) in vocab {
            let id = id.as_u64().ok_or("vocab id")? as usize;
            max_id = max_id.max(id);
            let bytes: Option<Vec<u8>> = s.chars().map(|c| u2b.get(&c).copied()).collect();
            entries.push((id, bytes));
        }
        let mut specials = HashMap::new();
        let mut special_by_id = HashMap::new();
        for t in v["added_tokens"].as_array().ok_or("added_tokens missing")? {
            let id = t["id"].as_u64().ok_or("added id")? as u32;
            let content = t["content"].as_str().ok_or("added content")?.to_string();
            max_id = max_id.max(id as usize);
            specials.insert(content.clone(), id);
            special_by_id.insert(id, content);
        }
        let mut pieces = vec![None; max_id + 1];
        for (id, b) in entries {
            if !special_by_id.contains_key(&(id as u32)) {
                pieces[id] = b;
            }
        }
        Ok(Self { pieces, specials, special_by_id })
    }

    pub fn id(&self, special: &str) -> Option<u32> {
        self.specials.get(special).copied()
    }

    /// Decode text tokens (specials are skipped) to a UTF-8 string.
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut bytes = Vec::new();
        for &i in ids {
            if let Some(Some(b)) = self.pieces.get(i as usize) {
                bytes.extend_from_slice(b);
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_table_is_a_bijection() {
        let t = unicode_to_byte();
        assert_eq!(t.len(), 256);
        assert_eq!(t[&'Ġ'], b' ');
        assert_eq!(t[&'a'], b'a');
    }
}
