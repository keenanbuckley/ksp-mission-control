// launch_helpers.ks - helpers used by launch.ks.

@lazyGlobal off.

function staticFlameout {
    local myEngines is list().
    list engines in myEngines.
    for eng in myEngines {
        if eng:throttlelock and eng:flameout {
            return true.
        }
    }.
    return false.
}

function throttleForThrust {
    parameter targetThrust.
    parameter minThrottle is 0.0.

    local staticThrust is 0.
    local dynamicThrust is 0.

    local myEngines is list().
    list engines in myEngines.
    for eng in myEngines {
        if eng:throttlelock {
            set staticThrust to staticThrust + eng:thrust.
        }
        else {
            set dynamicThrust to dynamicThrust + eng:availableThrust.
        }
    }.

    if staticFlameout() {
        if dynamicThrust = 0 { return minThrottle. }
        local adjThrottle is targetThrust / dynamicThrust.
        return min(max(minThrottle, adjThrottle), 1.0).
    }
    else if dynamicThrust > 0 {
        local adjThrottle is (targetThrust - staticThrust) / dynamicThrust.
        return min(max(minThrottle, adjThrottle), 1.0).
    } else {
        return minThrottle.
    }
}

// State for throttleForQCeiling. The Lexicon is mutated in place on every
// call, so each throttle lock needs a fresh controller.
function qCeilingController {
    parameter qMaxAtm.            // dynamic-pressure ceiling, in atmospheres
    parameter minThrottle is 0.1.
    parameter tau is 2.           // s, time constant of the approach to qMax
    parameter tauFilter is 0.5.   // s, smoothing on the disturbance estimate

    return lexicon(
        "qMaxAtm", qMaxAtm,
        "minThrottle", minThrottle,
        "tau", tau,
        "tauFilter", tauFilter,
        "lastT", -1,
        "lastQ", 0,
        "b", 0,
        "bValid", false,
        "cmd", 1.0).
}

// Throttle that holds dynamic pressure at or below qMax.
//
// Thrust moves q only through along-track acceleration, so
// dq/dt = k * T + b with k = 2 q / (m v). Everything thrust can't change
// (drag, gravity, density falloff) is b, estimated each tick from the measured
// dq/dt minus the current thrust's share and low-passed. The commanded total
// thrust makes dq/dt = (qMax - q) / tau, so q approaches qMax from below and
// holds there. Units: q in atm, T in kN, m in t, v in m/s.
//
// throttleForThrust turns that total into a throttle, subtracting solid
// thrust first. Solids can't be throttled once lit, so if they alone push q
// past qMax the liquids sit at minThrottle and q follows the solids.
function throttleForQCeiling {
    parameter ctl.

    local t is time:seconds.
    local q is ship:dynamicpressure.
    local dt is t - ctl["lastT"].
    if ctl["lastT"] >= 0 and dt <= 0 { return ctl["cmd"]. }

    local lastQ is ctl["lastQ"].
    local stale is ctl["lastT"] < 0 or dt > 1.
    set ctl["lastT"] to t.
    set ctl["lastQ"] to q.
    if stale {
        set ctl["bValid"] to false.
        set ctl["cmd"] to 1.0.
        return 1.0.
    }

    // k vanishes on the pad (v ~ 0) and above the atmosphere (q ~ 0), where
    // there is no q to limit.
    local v is ship:airspeed.
    local k is 0.
    if v > 0 { set k to 2 * q / (ship:mass * v). }
    if k < 1e-12 {
        set ctl["cmd"] to 1.0.
        return 1.0.
    }

    local bRaw is (q - lastQ) / dt - k * ship:thrust.
    if ctl["bValid"] {
        set ctl["b"] to ctl["b"] + (bRaw - ctl["b"]) * min(1, dt / ctl["tauFilter"]).
    } else {
        set ctl["b"] to bRaw.
        set ctl["bValid"] to true.
    }

    local qdotTarget is (ctl["qMaxAtm"] - q) / ctl["tau"].
    local cmd is throttleForThrust((qdotTarget - ctl["b"]) / k, ctl["minThrottle"]).
    set ctl["cmd"] to cmd.
    return cmd.
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
