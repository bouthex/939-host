//! Sonido breve cuando un servidor termina de iniciar (opcional, desactivado al instalar).
//!
//! Original de 939 Host: tres notas cortas ascendentes («pi-pi-piim») con un timbre suave de
//! campanita, sintetizadas en el momento. No usa archivos de sonido externos ni música de
//! terceros. Volumen moderado y una sola vez por arranque.

const RATE: u32 = 44_100;

/// WAV (PCM 16 bits, mono) del aviso.
pub fn start_chime() -> Vec<u8> {
    // (frecuencia en Hz, duración en s, comienzo en s)
    let notes = [(783.99, 0.11, 0.0), (987.77, 0.11, 0.12), (1318.51, 0.42, 0.24)];
    let total = 0.70;
    let n = (RATE as f64 * total) as usize;
    let mut buf = vec![0f64; n];
    for (f, dur, at) in notes {
        let start = (at * RATE as f64) as usize;
        let len = (dur * RATE as f64) as usize;
        for i in 0..len.min(n.saturating_sub(start)) {
            let t = i as f64 / RATE as f64;
            let attack = (t / 0.006).min(1.0);
            let decay = (-t * if dur > 0.2 { 7.0 } else { 18.0 }).exp();
            let w = (2.0 * std::f64::consts::PI * f * t).sin() + 0.25 * (4.0 * std::f64::consts::PI * f * t).sin();
            buf[start + i] += w * attack * decay;
        }
    }
    let peak = buf.iter().fold(0f64, |m, x| m.max(x.abs())).max(1e-9);
    let gain = 0.30 / peak;
    let mut pcm = Vec::with_capacity(n * 2);
    for x in buf { pcm.extend_from_slice(&(((x * gain).clamp(-1.0, 1.0) * i16::MAX as f64) as i16).to_le_bytes()); }
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(&pcm);
    w
}

/// Reproduce el aviso sin bloquear. En Windows usa el sonido del sistema (winmm, desde memoria).
pub fn play_start_chime() {
    #[cfg(windows)]
    {
        use once_cell::sync::Lazy;
        // La reproducción es asíncrona: los datos tienen que seguir vivos mientras suena.
        static WAV: Lazy<Vec<u8>> = Lazy::new(start_chime);
        unsafe {
            use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
            PlaySoundW(WAV.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wav_valido_breve_y_discreto() {
        let w = super::start_chime();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        let samples: Vec<i16> = w[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        let secs = samples.len() as f64 / 44_100.0;
        assert!(secs < 1.0, "breve: {secs} s");
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak < (i16::MAX as u16) / 3 + 100, "volumen moderado (pico {peak})");
        assert!(peak > 1000, "se escucha");
    }
}
