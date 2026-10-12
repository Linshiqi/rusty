#include "driver.h"

/* C decides the shape of the frame and asks Rust for the value.
 *
 * Deliberately arranged so the callback's result is visible: if
 * `rust_brightness` were not linked, or were called with the wrong
 * convention, the mask would be constant rather than moving. */
uint8_t driver_frame(uint32_t tick) {
    uint8_t level = rust_brightness(tick);
    uint8_t mask = 0;
    if (level > 0)   mask |= 1u << 0;
    if (level > 85)  mask |= 1u << 1;
    if (level > 170) mask |= 1u << 2;
    return mask;
}
