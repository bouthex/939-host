//! Actualizaciones de 939 Host.
//!
//! Principios de TUF (The Update Framework) adaptados a un proyecto con un solo propietario y sin
//! servidor propio: raíz de confianza con umbral y rotación encadenada, metadatos con vencimiento y
//! secuencia creciente, tamaño y SHA-256 firmados de cada archivo, separación entre compilar (CI),
//! autorizar (el propietario, en su PC) y distribuir (cualquier servidor de archivos). Ver
//! `docs/ACTUALIZACIONES.md`.

pub mod client;
pub mod download;
pub mod install;
pub mod keys;
pub mod metadata;
pub mod publish;
#[cfg(windows)]
pub mod cng;

pub use client::{Client, Decision, State};
pub use metadata::{Artifact, Envelope, Release, Root};
