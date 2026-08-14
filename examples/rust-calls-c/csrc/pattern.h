#ifndef PATTERN_H
#define PATTERN_H

#include <stdint.h>

/* One step of a three-lamp sweep: bit 0..2 are the lamps, 1 = lit.
 *
 * Stands in for the C you actually have — a vendor driver, a control loop
 * somebody validated years ago. The point is not the algorithm; it is that
 * the state lives on the C side and Rust never touches it. */
uint8_t pattern_step(void);

#endif /* PATTERN_H */
