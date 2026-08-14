#include "pattern.h"

/* Bounces 0,1,2,1,… — a sweep rather than a counter, so a wrong call
 * convention shows up as a visibly wrong pattern instead of an off-by-one
 * nobody notices. */
static uint8_t index_;
static int8_t  step_ = 1;

uint8_t pattern_step(void) {
    uint8_t lit = (uint8_t)(1u << index_);
    if (index_ == 2 && step_ == 1) {
        step_ = -1;
    } else if (index_ == 0 && step_ == -1) {
        step_ = 1;
    }
    index_ = (uint8_t)(index_ + step_);
    return lit;
}
