# Launch script

`launch.ks` takes the active vessel from the pad to a target apoapsis, then cuts off and
coasts out of the atmosphere ready for circularization. This document explains how the
ascent works and what every input parameter does.

The ascent is regime-aware: it changes control strategy based on where the vehicle is in
the atmosphere, not on hand-tuned velocity thresholds. The goal is a program whose knobs map
to physical quantities (a dynamic-pressure ceiling, an angle-of-attack budget, a
time-to-apoapsis floor) that transfer across vehicles and bodies, rather than vehicle-specific
numbers like "turn rate" that have to be re-found for every rocket.

## How it works

### The computation split

Anything derivable from the mission parameters plus body data is computed once, server-side,
in [src/launch_planning.rs](../src/launch_planning.rs) and shipped to the kOS script as
constants. The kerboscript runs no math at startup; it reads its config and runs the
real-time loop. The derived constants are `terrainMax`, `vMin`, `altPhase2Entry`, and
`terminalPitch` (see the derived-values section below). This keeps the analytical work
(vis-viva, the atmospheric-density solve) in Rust where it is unit-tested, and lets the
kerboscript focus on live vessel access.

### Throttle: max throttle clamped by qMax

Throttle is full unless dynamic pressure would exceed `qMax`, in which case it scales down by
`qMax / q` so dynamic pressure converges toward the cap. There is no target-TWR throttling:
full thrust is the default, and the only thing that pulls it back is the dynamic-pressure
ceiling. Solid boosters cannot throttle, so the ratio applies to the liquid engines only; if
solids alone push past `qMax`, the liquids drop to `throttleMin` and q is whatever the solids
produce until they burn out.

### Pitch: three phases on one cascade controller

Pitch is closed-loop. Rather than commanding a fixed attitude, the controller tracks the
velocity vector (prograde) and noses the body off it by a bounded angle to steer where
prograde goes next:

```
commanded_pitch = prograde_pitch + alpha_cmd
alpha_cmd       = clamp(target_pitch - prograde_pitch, -alpha_max, +alpha_max)
```

`alpha_cmd` is the angle of attack the controller spends to drag prograde toward
`target_pitch`. Bounding it by `alpha_max` is what keeps angle of attack (and therefore drag
and structural load) under control. This is self-correcting: as prograde rotates toward the
body, the error shrinks and `alpha_cmd` relaxes. Yaw is open-loop throughout (the launch
azimuth from spherical trig); only pitch is controlled, because at liftoff prograde is
vertical and has no heading to correct, and once the gravity turn develops it naturally
aligns with the azimuth.

The phases differ only in where `target_pitch` comes from and how large `alpha_max` is:

- **Phase 0, vertical climb.** Hold vertical under SAS. Exit once altitude clears the local
  terrain around the launch site AND there is enough airspeed for aerodynamic control
  authority (or the vehicle is already past the dense-atmosphere regime, in which case phase 1
  is skipped). Clearing only local terrain (rather than the body's tallest peak) keeps the
  vertical climb short so the gravity turn starts early, while there is still low-q AoA
  authority to establish it.
- **Phase 1, gravity turn.** Active in the high-density regime. `target_pitch` follows a
  smooth cosine profile from `initialProfilePitch` down to a predicted `terminalPitch`, keyed
  on altitude. `alpha_max` tapers with dynamic pressure (`alphaMax(q)`): generous at low q,
  tight near `qMax`. There is no separate "kick" event; the controller's first tick after
  phase 0 nudges the body a few degrees off vertical and the turn develops from there. If the
  active stage's thrust-to-weight drops below 1 mid-phase, it falls back to phase-2 behavior
  early so time-to-apoapsis does not collapse.
- **Phase 2, pitch_min through cutoff.** Active in the low-density regime. `target_pitch` is
  `pitch_min`, the shallowest prograde pitch that still holds time-to-apoapsis at
  `tApTarget`: `pitch_min = arcsin(g * tApTarget / v)` (vertical while v is too low for a
  valid arcsin). The `alpha_max` clamp is released since q is low, so the body slews freely.
  This formula self-adapts to thrust: a high-thrust stage reaches high v fast and gets a
  small `pitch_min` (turns toward horizontal quickly); a low-thrust stage stays pitched up
  longer, protecting apoapsis. Cutoff is `apoapsis > finalAltitude`.

The phase-1 / phase-2 boundary is physics-derived, not tuned. The server solves for the
altitude where atmospheric density crosses a threshold derived from `finalAltitude` and
`qMax`, and ships that altitude (`altPhase2Entry`). Because density falls monotonically with
altitude, the kOS script just compares its current altitude to that number.

### Preserved scaffolding

The countdown and launch-clamp release, auto-staging on flameout, the AG1 fairing trigger,
the coast-to-vacuum gate, the post-coast apoapsis touch-up burn, and the final hand-back of
control are unchanged from earlier versions. See the Rocket-design notes in the
[README](../README.md) for the clamp/staging/AG1 assumptions the vessel must satisfy.

## Input parameters

These are the user-facing mission parameters. The dashboard Launch button and the
[send_launch example](../examples/send_launch.rs) send them; the server computes the derived
constants and forwards everything to kOS. Defaults target a healthy Kerbin rocket going to
80 km LKO.

| Parameter | Units | Default | Reasonable range | What it does |
|---|---|---|---|---|
| `finalAltitude` | m | 80000 | 75000 to 250000 | Target apoapsis. Ascent cuts off when apoapsis reaches this. Must be above the atmosphere (Kerbin: > 70 km) for the coast gate to clear. |
| `targetInclination` | deg | 0 | 0 to 90 (magnitude >= launch latitude) | Target orbital inclination. 0 is equatorial (due east from KSC). The launch azimuth is clamped if the value is unreachable from the launch latitude. |
| `targetLan` | deg | -1 | -1, or 0 to 360 | Longitude of ascending node to time the launch to. Negative launches immediately; otherwise the script timewarps to the window. |
| `qMax` | kPa | 20 | 15 to 45 | Dynamic-pressure ceiling. Throttle scales back above this. Lower is gentler on the airframe but costs ascent speed; higher is more aggressive. Draggy or fragile stacks want lower; clean, sturdy ones tolerate higher. |
| `qAuth` | kPa | 0.7 | 0.4 to 1.5 | Aerodynamic-authority floor used to set the phase-0 airspeed gate (`vMin = sqrt(2 qAuth / rho_launch)`). Higher means climb faster (and a bit higher) before starting the turn; useful for fin-light or unstable designs. |
| `qHigh` | kPa | 20 | ~= qMax | Dynamic pressure at which the angle-of-attack budget reaches its floor. Usually set equal to `qMax`. Lowering it makes the controller cautious about AoA earlier in the climb. |
| `aMaxHigh` | deg | 10 | 5 to 15 | Maximum angle of attack the controller will spend at low q (the most authority it has to shape the turn). Higher turns more aggressively; too high risks drag and control losses if q is still meaningful. |
| `aMaxLow` | deg | 2 | 1 to 4 | Minimum angle of attack near `qHigh`. Keeps a little authority to trim drift at high q without spending much AoA. |
| `initialProfilePitch` | deg | 85 | 80 to 88 | Pitch the phase-1 profile starts at (degrees above horizon, so 85 is 5 degrees off vertical). Lower starts the gravity turn more aggressively. Higher is gentler; good for low-TWR or tippy rockets. |
| `tApTarget` | s | 30 | 20 to 60 | Time-to-apoapsis floor that `pitch_min` holds in phase 2. Higher buys margin against thrust drops (low-TWR upper stages, flameouts) at the cost of more gravity loss; lower is closer to an optimal trajectory. Raise it for a known marginal stage. |
| `throttleMin` | 0..1 | 0.1 | 0.05 to 0.25 | Throttle floor for the qMax controller, so engines with a high minimum-throttle threshold do not flame out when q clamps the throttle down. |

### Tuning notes

- **Start with qMax and tApTarget.** They have the largest effect. If the rocket loses parts
  or wobbles in the low atmosphere, drop `qMax`. If apoapsis or time-to-apoapsis collapses on
  an upper stage, raise `tApTarget`.
- **The AoA budget (`aMaxHigh`, `aMaxLow`, `qHigh`) shapes phase 1 only.** Increase `aMaxHigh`
  for a snappier gravity turn on a stable rocket; decrease it if the ascent looks twitchy near
  max-Q.
- **`initialProfilePitch` and `qAuth` control how the turn begins.** A higher
  `initialProfilePitch` and higher `qAuth` together mean "go straighter up for longer before
  committing to the turn," which suits underpowered or aerodynamically marginal designs.
- **Defaults are Kerbin-LKO.** Other bodies work without special-casing (a vacuum body skips
  phase 1 automatically), but the q-based knobs assume a Kerbin-like atmosphere; expect to
  retune `qMax` and `qAuth` for thicker or thinner air.

## Derived values (computed for you)

The server computes these from the parameters above plus live body data and sends them to
kOS. They are not inputs, but knowing them helps when reasoning about behavior:

| Value | Meaning |
|---|---|
| `terrainMax` | Max terrain height sampled on rings around the launch site, plus a clearance margin. Phase 0 holds vertical until the vehicle clears this, so a pad ringed by hills cannot start the turn into terrain. Local rather than body-global so a flat coastal pad starts the turn low instead of clearing a distant peak it never overflies. |
| `vMin` | Airspeed that produces `qAuth` at launch-site density. The phase-0 velocity gate. On airless bodies it is set unreachably high so phase 0 exits on altitude alone. |
| `altPhase2Entry` | Altitude where atmospheric density crosses the phase-1 / phase-2 threshold (`2 qMax / v_upper^2`, solved against the body's real density curve). The gravity-turn-to-pitch_min handoff altitude. |
| `terminalPitch` | Predicted `pitch_min` at `altPhase2Entry`, used as the phase-1 cosine profile's endpoint so the handoff to phase 2 is smooth. A rough estimate; any error is absorbed by the cascade controller, which has wide authority at the low q of the handoff. |

## Changing the parameters

The dashboard Launch button sends a fixed default set (see the handler in
[static/index.html](../static/index.html)). To launch with different values, edit that
object, or use the [send_launch example](../examples/send_launch.rs), which builds the same
request from `LaunchParams` and can be edited to taste. Any parameter omitted from the
request falls back to the default in
[`LaunchParams::default`](../src/launch_planning.rs).
