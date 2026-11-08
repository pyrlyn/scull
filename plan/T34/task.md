# T34. Decode PNG through the shared rgba-png crate

scull-image's `raster::decode_png` and `widen` moved to the shared `rgba-png` crate (pyrlyn/crates, also used by slint-bindings). `decode_png` keeps its order: header first, `rgba_len` on the size before any pixel buffer, then the capped decode; png errors still map to `ImageError::Decode`. Test fixtures encode through `rgba_png::encode_png`, so they now carry an sRGB chunk; `png` stays as a dev-dependency for the grey and palette fixtures.
