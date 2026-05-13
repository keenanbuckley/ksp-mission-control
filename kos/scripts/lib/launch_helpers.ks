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
