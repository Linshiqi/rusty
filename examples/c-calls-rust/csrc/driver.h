#ifndef DRIVER_H
#define DRIVER_H

#include <stdint.h>

/* Drives one frame and returns the lamp mask it chose.
 *
 * This is the C that owns the loop's logic; it calls back into Rust for the
 * part Rust owns. Both declarations live here so the boundary is one file
 * somebody can read. */
uint8_t driver_frame(uint32_t tick);

/* Implemented in Rust, linked from this same binary. For an API larger than
 * one function, generate this with cbindgen rather than maintaining two
 * declarations of the same thing by hand. */
uint8_t rust_brightness(uint32_t tick);

#endif /* DRIVER_H */
