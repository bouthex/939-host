//! Nombre visible de cada PC: aleatorio al principio, editable y **solo visual**.
//! La identidad real de la PC es su clave P2P (`identity`); cambiar el nombre nunca la toca,
//! ni las claves, ni los permisos, ni la identidad permanente de los servidores.

/// Nombres con personalidad, sin caer en lo infantil: cosas de acá y del mundo de bloques.
pub const NAMES: &[&str] = &[
    "Mate", "Pampa", "Cuadradín", "Bloquito", "Bizcochito", "Creeper", "Enderman",
    "Alfajor", "Tero", "Ombú", "Hornero", "Ceibo", "Quebracho", "Yerba", "Tranquera", "Aljibe",
    "Pulpería", "Faro", "Médano", "Cardón", "Puna", "Ladrillo", "Adoquín", "Obsidiana", "Cuarzo",
    "Andesita", "Redstone", "Brújula", "Linterna", "Farol", "Yunque", "Piquito", "Antorcha",
    "Cofre", "Lingote", "Esmeralda", "Bambú", "Musgo", "Calcita", "Pizarra", "Basalto", "Baliza",
];

/// Símbolos que acompañan al nombre (se dibujan en la interfaz).
pub const SYMBOLS: &[&str] = &["bloque", "pico", "antorcha", "hoja", "cofre", "gema", "faro", "montana", "luna", "rayo", "ola", "estrella"];

fn rnd(n: usize) -> usize {
    let mut b = [0u8; 4];
    let _ = getrandom::getrandom(&mut b);
    u32::from_le_bytes(b) as usize % n.max(1)
}

pub fn random_name() -> String { NAMES[rnd(NAMES.len())].to_string() }
pub fn random_symbol() -> String { SYMBOLS[rnd(SYMBOLS.len())].to_string() }

/// Normaliza un nombre editado: sin caracteres de control, espacios colapsados, 1–24 caracteres.
pub fn clean_name(raw: &str) -> Option<String> {
    let s: String = raw.chars().filter(|c| !c.is_control()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.chars().take(24).collect();
    (!s.is_empty()).then_some(s)
}

pub fn valid_symbol(s: &str) -> bool { SYMBOLS.contains(&s) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_clean_and_short() {
        assert_eq!(clean_name("  Mi   PC\n "), Some("Mi PC".into()));
        assert_eq!(clean_name("\t"), None);
        assert_eq!(clean_name(&"x".repeat(40)).unwrap().chars().count(), 24);
        assert!(NAMES.iter().all(|n| clean_name(n).as_deref() == Some(*n)));
        assert!(valid_symbol(&random_symbol()) && NAMES.contains(&random_name().as_str()));
        assert!(!valid_symbol("../x"));
    }
}
