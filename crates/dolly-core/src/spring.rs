//! Critically damped spring integrator.
//!
//! Everything smooth in Dolly — the cursor glide, the camera pan, the zoom
//! ease — is a critically damped spring chasing a target. Critical damping is
//! the sweet spot: fastest possible convergence with zero overshoot, which is
//! exactly what a "human camera operator" feel requires. (Underdamped = bouncy
//! GoPro; overdamped = laggy screen-share.)
//!
//! Integration is semi-implicit Euler with a fixed internal substep so results
//! are stable and deterministic regardless of the caller's frame rate.

/// A 1-D critically damped spring.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    /// Natural angular frequency ω (rad/s). Higher = snappier.
    pub omega: f64,
    pub position: f64,
    pub velocity: f64,
}

/// Internal fixed substep (seconds). 1 kHz keeps the integrator stable for any
/// realistic ω (we clamp ω ≤ 200 rad/s) while staying cheap.
const SUBSTEP: f64 = 0.001;
const MAX_OMEGA: f64 = 200.0;

impl Spring {
    pub fn new(omega: f64, initial: f64) -> Self {
        Self {
            omega: omega.clamp(0.0, MAX_OMEGA),
            position: initial,
            velocity: 0.0,
        }
    }

    /// Advance the spring by `dt` seconds toward `target`.
    /// Returns the new position.
    pub fn step(&mut self, dt: f64, target: f64) -> f64 {
        if dt <= 0.0 {
            return self.position;
        }
        // Uniform substeps (last partial step folded in evenly) for stability.
        let n = (dt / SUBSTEP).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..(n as u64) {
            // Critically damped: acceleration = -ω²·(x − target) − 2ω·v
            let accel =
                -self.omega * self.omega * (self.position - target) - 2.0 * self.omega * self.velocity;
            self.velocity += accel * h;
            self.position += self.velocity * h;
        }
        self.position
    }

    /// Hard-set state (e.g. when a cut/trim teleports the camera).
    pub fn snap_to(&mut self, value: f64) {
        self.position = value;
        self.velocity = 0.0;
    }
}

/// A 2-D spring (two independent 1-D springs). Used for cursor position and
/// camera center.
#[derive(Debug, Clone, Copy)]
pub struct Spring2 {
    pub x: Spring,
    pub y: Spring,
}

impl Spring2 {
    pub fn new(omega: f64, ix: f64, iy: f64) -> Self {
        Self {
            x: Spring::new(omega, ix),
            y: Spring::new(omega, iy),
        }
    }

    pub fn step(&mut self, dt: f64, tx: f64, ty: f64) -> (f64, f64) {
        (self.x.step(dt, tx), self.y.step(dt, ty))
    }

    pub fn snap_to(&mut self, x: f64, y: f64) {
        self.x.snap_to(x);
        self.y.snap_to(y);
    }

    pub fn position(&self) -> (f64, f64) {
        (self.x.position, self.y.position)
    }

    pub fn speed(&self) -> f64 {
        (self.x.velocity * self.x.velocity + self.y.velocity * self.y.velocity).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_to_target() {
        let mut s = Spring::new(12.0, 0.0);
        for _ in 0..120 {
            s.step(1.0 / 60.0, 100.0);
        }
        assert!((s.position - 100.0).abs() < 0.5, "pos = {}", s.position);
    }

    #[test]
    fn critical_damping_does_not_overshoot() {
        let mut s = Spring::new(12.0, 0.0);
        let mut max_pos: f64 = 0.0;
        for _ in 0..600 {
            max_pos = max_pos.max(s.step(1.0 / 60.0, 100.0));
        }
        // Numerical integration allows a hair of overshoot; anything visible
        // (>1% of travel) would read as bounce on screen.
        assert!(max_pos <= 101.0, "overshoot: {}", max_pos);
    }

    #[test]
    fn deterministic_across_frame_rates() {
        // 30 fps caller and 120 fps caller must land in the same place,
        // because internally we integrate on a fixed substep.
        let mut a = Spring::new(10.0, 0.0);
        let mut b = Spring::new(10.0, 0.0);
        for _ in 0..30 {
            a.step(1.0 / 30.0, 50.0);
        }
        for _ in 0..120 {
            b.step(1.0 / 120.0, 50.0);
        }
        // Sub-centipixel agreement between 30 and 120 fps callers.
        assert!((a.position - b.position).abs() < 1e-2);
    }

    #[test]
    fn zero_dt_is_noop() {
        let mut s = Spring::new(10.0, 5.0);
        assert_eq!(s.step(0.0, 100.0), 5.0);
    }
}
