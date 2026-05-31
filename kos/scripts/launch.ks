// launch.ks handles launching a vessel into orbit, stopping at target apoapsis.
//
// Triggered via dispatch_listener.ks's `run_script` op with a single config
// Lexicon (keys below). The host (src/launch_planning.rs) pre-computes the
// derived constants (terrainMax, vMin, altPhase2Entry, terminalPitch); this
// script reads them and runs the real-time control loop with no startup math.
//
// Ascent is three phases on one pitch-only cascade controller. Phase 0 climbs
// vertically until terrain is cleared and there's airspeed for control
// authority. Phase 1 is the high-density gravity turn: an altitude-keyed cosine
// pitch profile tracked with an AoA-bounded cascade. Phase 2 (low density)
// commands pitch_min through cutoff with the AoA clamp released. Yaw is
// open-loop launchAzimuth throughout.

@lazyGlobal off.

parameter cfg.

local finalAltitude       is cfg["finalAltitude"].
local targetInclination   is cfg["targetInclination"].
local targetLan           is cfg["targetLan"].
local aMaxHigh            is cfg["aMaxHigh"].
local aMaxLow             is cfg["aMaxLow"].
local initialProfilePitch is cfg["initialProfilePitch"].
local tApTarget          is cfg["tApTarget"].
local throttleMin        is cfg["throttleMin"].
local terrainMax         is cfg["terrainMax"].
local vMin               is cfg["vMin"].
local altPhase2Entry     is cfg["altPhase2Entry"].
local terminalPitch      is cfg["terminalPitch"].

// ship:dynamicpressure is in atmospheres; qMax/qHigh arrive in kPa.
local kpaToAtm  is 1 / constant:atmtokpa.
local qMaxAtm   is cfg["qMax"]  * kpaToAtm.
local qHighAtm  is cfg["qHigh"] * kpaToAtm.

runOncePath("/lib/launch_helpers.ks").

print "RUNNING launch (alt=" + finalAltitude + ", inc=" + targetInclination + ", qMax=" + cfg["qMax"] + " kPa).".

local compassHeading is launchAzimuth(targetInclination, finalAltitude).

if targetLan >= 0 {
    print "Timewarping to launch window.".
    local launchEta is (ship:body:rotationPeriod / 360.0) * (targetLan - ship:geoposition:lng - ship:body:rotationAngle).
    until launchEta > 0 {
        set launchEta to launchEta + ship:body:rotationPeriod.
    }
    kuniverse:timewarp:warpto(time:seconds + launchEta - 3).
    wait launchEta - 3.
}

// Number of staging events needed to clear all launch clamps, counting from
// the current stage down to (and including) the lowest-numbered clamp stage.
// Spread across the countdown so clamps release at T-0. Any pre-clamp
// stages (typically main-engine ignition) fire on the ticks before it.
function stagesToClamp {
    local minClampStage is 999.
    for p in ship:parts {
        if p:modules:contains("LaunchClamp") and p:stage < minClampStage {
            set minClampStage to p:stage.
        }
    }
    if minClampStage = 999 { return 0. }
    return stage:number - minClampStage.
}

local stagingEvents is stagesToClamp().
local countdownStart is max(stagingEvents, 3).

from {local t is countdownStart.} until t < 0 step {set t to t - 1.} do {
    if t = stagingEvents {
        // Full throttle so engines ignite during countdown. The qMax-controlled
        // lock below replaces this once the countdown completes.
        print "" + t + ". Throttling up.".
        lock throttle to 1.0.
    }
    else if t < stagingEvents {
        print "" + t + ". Staging.".
        wait until stage:ready.
        stage.
    } else {
        print "" + t + ".".
    }
    if t > 0 { wait 1. }
}

when maxThrust = 0 or engineFlameout() then {
    print "Staging.".
    stage.
    wait until stage:ready.
    wait 0.
    preserve.
}

when ship:velocity:surface:mag > 1000 and ship:dynamicpressure < 0.01 then {
    ag1 on.
}

lock throttle to throttleForQMax(qMaxAtm, throttleMin).

// Pitch elevation (deg above the local horizon) of the surface-velocity vector.
// Returns vertical while velocity is undefined so phase-0 handoff has a value.
function progradePitch {
    local v is ship:velocity:surface.
    if v:mag < 0.01 { return 90. }
    return 90 - vAng(ship:up:vector, v:normalized).
}

// AoA-spending budget: high at low q, tapering to aMaxLow as q approaches qHigh.
function alphaMax {
    parameter qAtm.
    if qHighAtm <= 0 { return aMaxLow. }
    local frac is max(0, min(1, qAtm / qHighAtm)).
    return aMaxLow + (aMaxHigh - aMaxLow) * (1 - frac).
}

// Minimum prograde pitch that holds time-to-apoapsis at tApTarget. Saturates to
// vertical while v is too low for a valid arcsin. arcsin returns degrees in kOS.
function pitchMin {
    parameter vmag.
    local g is body:mu / ((body:radius + ship:altitude)^2).
    if vmag <= g * tApTarget { return 90. }
    return arcsin(g * tApTarget / vmag).
}

// Shared cascade tick: nose the body off prograde by alphaCmd to drive prograde
// toward targetPitch, with alphaCmd clamped to +/- aMax. Self-correcting: as
// prograde rotates toward the body the error shrinks and alphaCmd relaxes.
local alphaCmd is 0.
local cmdPitch is 90.

function cascadeTick {
    parameter targetPitch.
    parameter aMax.
    local pp is progradePitch().
    local err is targetPitch - pp.
    set alphaCmd to max(-aMax, min(aMax, err)).
    set cmdPitch to pp + alphaCmd.
}

// Phase 0: vertical climb. Exit once terrain is cleared and there's either
// airspeed for control authority or no dense gravity turn ahead (then phase 1
// skips and we drop straight into phase 2).
sas on.
print "Phase 0: vertical climb.".
until ship:altitude > terrainMax and (ship:velocity:surface:mag > vMin or ship:altitude > altPhase2Entry) {
    wait 0.
}
sas off.
set cmdPitch to progradePitch().
lock steering to heading(compassHeading, cmdPitch).

// Phase 1: high-density gravity turn. Skipped on thin/no atmosphere (already
// past the regime boundary at phase-0 exit). The boundary is altitude-based:
// density falls monotonically with altitude, so altitude > altPhase2Entry is
// equivalent to the rho < rho_threshold regime split the host solved for.
if ship:altitude < altPhase2Entry {
    print "Phase 1: gravity turn.".
    until ship:altitude >= altPhase2Entry {
        local span is altPhase2Entry - terrainMax.
        local s is 0.
        if span > 0 {
            set s to max(0, min(1, (ship:altitude - terrainMax) / span)).
        }
        // kOS trig takes degrees, so cos(180*s) is the cosine ease over s in [0,1].
        local tp is initialProfilePitch + (terminalPitch - initialProfilePitch) * (1 - cos(180 * s)) / 2.

        // Stage-TWR fallback: a stage that can't hold the profile (TWR < 1)
        // gets phase-2 treatment early so t_ap doesn't collapse mid-turn.
        local g is body:mu / ((body:radius + ship:altitude)^2).
        if ship:availableThrust < ship:mass * g {
            cascadeTick(pitchMin(ship:velocity:surface:mag), 90).
        } else {
            cascadeTick(tp, alphaMax(ship:dynamicpressure)).
        }
        wait 0.
    }
}

// Phase 2: pitch_min policy through cutoff. q is low here, so the AoA clamp is
// released (aMax = 90) and the body slews freely to drive prograde to pitch_min.
print "Phase 2: pitch_min policy.".
until ship:apoapsis > finalAltitude {
    cascadeTick(pitchMin(ship:velocity:surface:mag), 90).
    wait 0.
}

print "Reached apoapsis of " + round(ship:apoapsis) + " m, cutting throttle.".
print "Coasting to " + round(ship:body:atm:height) + " m.".
lock throttle to 0.
wait until ship:altitude > ship:body:atm:height.

if ship:apoapsis < finalAltitude {
    print "Burning to apoapsis.".
    kuniverse:timewarp:cancelwarp().
    wait until kuniverse:timewarp:isSettled().
    lock steering to prograde.
    lock throttle to throttleForQMax(qMaxAtm, throttleMin).
    wait until ship:apoapsis > finalAltitude.
    lock throttle to 0.
}

// clear player's throttle so handing control back doesn't snap throttle on
set ship:control:pilotMainThrottle to 0.

unlock steering.
sas on.

print "Target apoapsis reached. Ready for circularization.".
