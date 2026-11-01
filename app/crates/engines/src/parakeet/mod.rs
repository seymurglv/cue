//! Parakeet TDT on this computer (spec "Local speech recognition"): the log-mel features ([`mel`]), the vocabulary
//! ([`vocab`]) and the TDT greedy decoding ([`tdt`]) are CUE's own code, ported from parakeet-rs (MIT/Apache-2.0);
//! the encoder and the decoder/joint run in ONNX Runtime.
pub mod mel;
pub mod model;
pub mod tdt;
pub mod vocab;
