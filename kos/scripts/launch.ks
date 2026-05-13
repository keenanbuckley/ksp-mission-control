// launch.ks handles launching a vessel into orbit, stopping at target apoapsis.
//
// Triggered via dispatch_listener.ks's `run_script` op with a 6-element args
// list (positional, in parameter order below).

@lazyGlobal off.

// defaults work well when launching from KSC
parameter finalAltitude is 80000.
parameter targetInclination is 0.
parameter turnRate is 12.
parameter targetTWR is 2.0.
parameter initialSpeed is 100.
parameter targetLan is -1.   // launch-window LAN target; negative = launch immediately

runOncePath("/lib/launch_helpers.ks").

print "RUNNING launch (alt=" + finalAltitude + ", inc=" + targetInclination + ", twr=" + targetTWR + ").".

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
        // Full throttle so engines ignite during countdown. The TWR-controlled
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

lock gravAcc to body:mu/((body:radius + ship:altitude)^2).
lock weight to gravAcc * ship:mass.
lock throttle to throttleForThrust(targetTWR * weight).

sas on.
local yaw is compassHeading.
local pitch is 90.

when ship:velocity:surface:mag > initialSpeed then {
    sas off.
    lock steering to heading(yaw, pitch).
}

local phase is "init".
until ship:apoapsis > finalAltitude {
    if ship:velocity:surface:mag < initialSpeed {
        if phase <> "accel" {
            print "Accelerating to " + initialSpeed + " m/s.".
            set phase to "accel".
        }
    } else if ship:velocity:surface:mag < 80*turnRate+initialSpeed {
        set pitch to max(90 - (ship:velocity:surface:mag-initialSpeed)/turnRate, 10).
        set yaw to compassHeading.
        if phase <> "pitch" {
            print "Pitching over.".
            set phase to "pitch".
        }
    }
}

print "Reached apoapsis of " + round(ship:apoapsis) + " m, cutting throttle.".
print "Coasting to " + round(ship:body:atm:height) + " m.".
lock throttle to 0.
wait until ship:altitude > ship:body:atm:height.

if ship:apoapsis < finalAltitude {
    print "Burning to apoapsis.".
    kuniverse:timewarp:cancelwarp().
    wait until kuniverse:timewarp:isSettled().
    lock throttle to throttleForThrust(targetTWR * weight).
    wait until ship:apoapsis > finalAltitude.
    lock throttle to 0.
}

// clear player's throttle so handing control back doesn't snap throttle on
set ship:control:pilotMainThrottle to 0.

unlock steering.
sas on.

print "Target apoapsis reached. Ready for circularization.".
