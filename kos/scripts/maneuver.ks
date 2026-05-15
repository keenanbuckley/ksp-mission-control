// maneuver.ks executes the next scheduled maneuver node and removes it.
//
// Triggered via dispatch_listener.ks's `run_script` op with an empty args list.

@lazyGlobal off.

runOncePath("/lib/maneuver_helpers.ks").

if not hasnode {
    print "maneuver: no scheduled node; aborting.".
} else {

print "RUNNING maneuver".

local nd is nextNode.

// calculate estimate of burn duration
local pressure is 0.
local flowRate is availableMassFlowRateAt(pressure).

print "current max flow rate: " + availableMassFlowRate().
print "vacuum max flow rate: " + flowRate.

local effectiveExhaustVelocity is ship:availableThrustAt(pressure)/flowRate.
local burnDuration is burnTime(ship:mass, nd:deltav:mag-1, effectiveExhaustVelocity, flowRate).

local nextMass1 is rocketEquationFinalMass(ship:mass, nd:deltav:mag-1, effectiveExhaustVelocity).
local flowRate1 is nextMass1/effectiveExhaustVelocity.
set burnDuration to burnDuration + burnTime(nextMass1, 0.9, effectiveExhaustVelocity, flowRate1).

local nextMass2 is rocketEquationFinalMass(nextMass1, 0.9, effectiveExhaustVelocity).
local flowRate2 is 0.1*flowRate1.
set burnDuration to burnDuration + burnTime(nextMass2, 0.1, effectiveExhaustVelocity, flowRate2).

local flowRateSum is flowRate + flowRate1 + flowRate2.
local burnStart is (flowRate/flowRateSum)*meanBurnTime(ship:mass, nd:deltav:mag-1, effectiveExhaustVelocity, flowRate).
set burnStart to burnStart + (flowRate1/flowRateSum)*meanBurnTime(nextMass1, 0.9, effectiveExhaustVelocity, flowRate1).
set burnStart to burnStart + (flowRate2/flowRateSum)*meanBurnTime(nextMass2, 0.1, effectiveExhaustVelocity, flowRate2).

print "Estimated burn duration of " + round(burnDuration, 3) + " seconds".
print "Crude Estimate: " + round(nd:deltav:mag/(ship:availablethrust/ship:mass), 3) + " seconds".
print "Starting burn " + round(burnStart, 3) + " seconds before node ETA".

// wait until 60 seconds before burn
wait until nd:eta <= burnStart + 60.

// turn to face the direction the rocket's velocity is changing in
local dv0 is nd:deltaV.
sas off.
lock steering to dv0.

// wait until rocket is facing the right direction
wait until vang(dv0, ship:facing:vector) < 0.25.

// wait until burn start
wait until nd:eta <= burnStart + 10.
kuniverse:timewarp:cancelwarp().
wait until kuniverse:timewarp:isSettled().
wait until nd:eta <= burnStart.

// create a setpoint we can manipulate
local throttleSetpoint is 0.
lock throttle to throttleSetpoint.

// execute maneuver node
lock steering to nd:deltav.
set throttleSetpoint to 1.

wait until nd:deltaV:mag < 1 or vDot(dv0, nd:deltav) < 0.
set throttleSetpoint to 1*ship:mass/ship:availableThrustAt(pressure).

wait until nd:deltaV:mag < 0.1 or vDot(dv0, nd:deltav) < 0.
set throttleSetpoint to 0.1*ship:mass/ship:availableThrustAt(pressure).

wait until nd:deltaV:mag < 0.01 or vDot(dv0, nd:deltav) < 0.
set throttleSetpoint to 0.

// print stats
print "End burn, remain dv " + round(nd:deltav:mag,1) + "m/s, vdot: " + round(vdot(dv0, nd:deltav),1).

// remove node so we can execute future ones
remove nd.

// unlock controls
unlock steering.
unlock throttle.

// clear player's throttle so handing control back doesn't snap throttle on
set ship:control:pilotMainThrottle to 0.

// turn on stability assist
sas on.

}
