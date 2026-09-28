// launch_helpers.ks - helpers used by launch.ks.

@lazyGlobal off.

// Lit solids. A per-stage snapshot: walking the engine list costs enough
// instructions at the default IPU to starve the mainline if done every tick.
// Solids light and burn out on stage events, so a stage change is the only
// time the set changes.
function litSolids {
    local myEngines is list().
    list engines in myEngines.
    local lit is list().
    for eng in myEngines {
        if eng:throttlelock and eng:ignition and not eng:flameout { lit:add(eng). }
    }
    return lit.
}

// State for throttleForQCeiling. Each throttle lock needs a fresh controller.
function qCeilingController {
    parameter qMaxAtm.            // dynamic-pressure ceiling, in atmospheres
    parameter minThrottle is 0.1.
    parameter tau is 2.           // s, time constant of the approach to qMax
    parameter gain is 0.25.       // fraction of the dq/dt error corrected per tick

    // Output is (qMax - q) / tau - dq/dt: the dq/dt error against the target
    // rate. kOS takes the derivative on the measurement, not the error.
    local pid is pidLoop(1 / tau, 0, 1).
    set pid:setpoint to qMaxAtm.
    return lexicon(
        "pid", pid,
        "minThrottle", minThrottle,
        "gain", gain,
        "stageNum", stage:number,
        "solids", litSolids()).
}

// Throttle that holds dynamic pressure at or below qMax.
//
// Thrust moves q only through along-track acceleration, so
// dq/dt = k * T + b with k = 2 q / (m v). Each tick the total thrust moves
// from the current thrust by a fraction of the dq/dt error over k, driving
// dq/dt to (qMax - q) / tau: q approaches qMax from below and holds there.
// The full correction would be deadbeat, and rings whenever the lock runs a
// tick late. Units: q in atm, T in kN, m in t, v in m/s.
//
// Solids can't be throttled once lit, so their thrust counts toward the total
// and the liquids make up the rest. If the solids alone push q past qMax, the
// liquids sit at minThrottle and q follows the solids.
function throttleForQCeiling {
    parameter ctl.

    if stage:number <> ctl["stageNum"] {
        set ctl["stageNum"] to stage:number.
        set ctl["solids"] to litSolids().
    }

    local qNow is ship:dynamicpressure.
    local qdotError is ctl["pid"]:update(time:seconds, qNow).

    // k vanishes on the pad (v ~ 0) and above the atmosphere (q ~ 0), where
    // there is no q to limit.
    local vAir is ship:airspeed.
    if vAir <= 0 { return 1.0. }
    local k is 2 * qNow / (ship:mass * vAir).
    if k <= 0 { return 1.0. }

    local solidThrust is 0.
    local liquidAvail is ship:availableThrust.
    if not ctl["solids"]:empty {
        for eng in ctl["solids"] {
            set solidThrust to solidThrust + eng:thrust.
            set liquidAvail to liquidAvail - eng:availableThrust.
        }
    }
    if liquidAvail <= 0 { return ctl["minThrottle"]. }

    local targetThrust is ship:thrust + ctl["gain"] * qdotError / k.
    return min(max(ctl["minThrottle"], (targetThrust - solidThrust) / liquidAvail), 1.0).
}

function engineFlameout {
    local myEngines is list().
    list engines in myEngines.
    for eng in myEngines {
        if eng:flameout {
            return true.
        }
    }.
    return false.
}

function launchAzimuth {
    parameter targetInclination.
    parameter targetAltitude is 80000.
    parameter launchLatitude is ship:geoPosition:lat.
    parameter orbitBody is body.

    // Inertial azimuth from spherical trig: cos(i) = cos(lat) * sin(az)
    local sinAz is cos(targetInclination) / cos(launchLatitude).
    local inertialAzimuth is arcsin(min(1, max(-1, sinAz))).

    // Orbital velocity for a circular orbit at target altitude (vis-viva)
    local targetRadius is orbitBody:radius + targetAltitude.
    local vOrbit is sqrt(orbitBody:mu / targetRadius).

    // Surface rotation velocity at launch latitude
    local vRot is (2 * constant:pi * orbitBody:radius * cos(launchLatitude)) / orbitBody:rotationPeriod.

    // Subtract rotation from inertial velocity to get surface-relative heading
    local vXrot is vOrbit * sin(inertialAzimuth) - vRot.
    local vYrot is vOrbit * cos(inertialAzimuth).

    local azimuth is arctan2(vXrot, vYrot).
    if azimuth < 0 { set azimuth to azimuth + 360. }
    return azimuth.
}
