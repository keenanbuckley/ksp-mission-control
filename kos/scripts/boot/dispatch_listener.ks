// dispatch_listener.ks - mc-link boot script.
//
// On boot: claim the "mc" part tag if no other holder exists, then drain
// the kIPC inbox and dispatch ops. Messages arrive as kerboscript Lexicons
// (kIPC has already deserialized the JSON envelope).

@lazyGlobal off.

print "dispatch_listener: waiting for ship to unpack.".
wait until ship:unpacked.

// archive is the working volume. scripts and helpers live there per the
// deploy tool's contract, so absolute paths like "/launch.ks" resolve.
switch to 0.

local MC_TAG           is "mc".
local PASSIVE_POLL     is 5.   // seconds between vessel-change re-checks
local HEARTBEAT_PERIOD is 1.   // real-time seconds between beats while a script runs

// Identifies this CPU session. The uid separates CPUs that boot in the same
// tick after a scene load; the realtime part changes on every reboot, which is
// how the server tells a reloaded CPU from the one that was running a script.
local BOOT_ID is core:part:uid + "/" + kuniverse:realtime.

local hbPath is "".   // script currently running, or "" while idle
local hbNext is 0.

// Argument shape each runnable script expects: "lexicon" takes a single config
// Lexicon, "none" takes no args.
local SCRIPT_SHAPE is lexicon(
    "launch.ks", "lexicon",
    "maneuver.ks", "none"
).

// Lifecycle events back to the server. kIPC serializes the Lexicon; the
// kRPC client receives a JSON envelope it decodes via control::decode_dict.
function sendEvent {
    parameter ev.
    ADDONS:KIPC:CONNECTION:SENDMESSAGE(ev).
}

function ackOp {
    parameter op.
    sendEvent(lexicon("kind", "command_ack", "op", op)).
}

function sendHeartbeat {
    sendEvent(lexicon(
        "kind", "heartbeat",
        "path", hbPath,
        "boot", BOOT_ID,
        "active", kuniverse:activevessel = ship
    )).
}

function rejectScript {
    parameter p, reason.
    print "dispatch_listener: " + reason + ": /" + p + "; dropping.".
    sendEvent(lexicon("kind", "script_done", "path", p, "ok", false, "reason", reason, "boot", BOOT_ID)).
}

function otherMcHolderExists {
    local self_uid is core:part:uid.
    for p in ship:parts {
        if p:tag = MC_TAG and p:uid <> self_uid {
            return true.
        }
    }
    return false.
}

function claimMc {
    set core:part:tag to MC_TAG.
    print "dispatch_listener: claimed mc tag on " + core:part:name + ".".
}

function toggleAg {
    parameter n.
    if      n = 1  { toggle ag1.  }
    else if n = 2  { toggle ag2.  }
    else if n = 3  { toggle ag3.  }
    else if n = 4  { toggle ag4.  }
    else if n = 5  { toggle ag5.  }
    else if n = 6  { toggle ag6.  }
    else if n = 7  { toggle ag7.  }
    else if n = 8  { toggle ag8.  }
    else if n = 9  { toggle ag9.  }
    else if n = 10 { toggle ag10. }
    else {
        print "dispatch_listener: ag n out of range: " + n + ".".
        return.
    }
    print "dispatch_listener: toggled AG" + n + ".".
}

function handleMessage {
    parameter content.
    if not content:istype("Lexicon") {
        print "dispatch_listener: bad message type; dropping.".
        return.
    }
    if not content:haskey("op") {
        print "dispatch_listener: no op field; dropping.".
        return.
    }
    local op is content:op.
    if op = "ping" {
        sendHeartbeat().
    } else if op = "toggle_ag" {
        if not content:haskey("n") {
            print "dispatch_listener: toggle_ag missing n; dropping.".
            return.
        }
        ackOp(op).
        toggleAg(content:n).
    } else if op = "add_node" {
        if not content:haskey("dv") {
            print "dispatch_listener: add_node missing dv; dropping.".
            return.
        }
        if not content:haskey("ut") {
            print "dispatch_listener: add_node missing ut; dropping.".
            return.
        }
        ackOp(op).
        local n is node(content:ut, 0, 0, content:dv).
        add n.
        print "dispatch_listener: added node at ut=" + content:ut + " dv=" + content:dv + ".".
    } else if op = "run_script" {
        if not content:haskey("path") {
            print "dispatch_listener: run_script missing path; dropping.".
            return.
        }
        local p is content:path.
        if not SCRIPT_SHAPE:haskey(p) {
            rejectScript(p, "unknown script").
            return.
        }
        if not exists("/" + p) {
            rejectScript(p, "script not found").
            return.
        }
        local shape is SCRIPT_SHAPE[p].
        if shape = "none" {
            ackOp(op).
            print "dispatch_listener: running /" + p + ".".
            set hbPath to p.
            set hbNext to 0.
            runPath("/" + p).
        } else if shape = "lexicon" {
            if not content:haskey("args") {
                rejectScript(p, "run_script missing args").
                return.
            }
            local a is content:args.
            if not a:istype("Lexicon") {
                rejectScript(p, "run_script args must be a lexicon").
                return.
            }
            ackOp(op).
            print "dispatch_listener: running /" + p + ".".
            set hbPath to p.
            set hbNext to 0.
            runPath("/" + p, a).
        } else {
            rejectScript(p, "unknown shape " + shape).
            return.
        }
        set hbPath to "".
        print "dispatch_listener: /" + p + " returned.".
        // A script that aborts never gets here; the server notices the
        // heartbeat stopping instead.
        sendEvent(lexicon("kind", "script_done", "path", p, "ok", true, "boot", BOOT_ID)).
    } else {
        print "dispatch_listener: unknown op '" + op + "'; dropping.".
    }
}

function runActive {
    print "dispatch_listener: active mode.".
    until false {
        wait until not core:messages:empty.
        until core:messages:empty {
            handleMessage(core:messages:pop():content).
        }
    }
}

function runPassive {
    print "dispatch_listener: passive mode (mc held elsewhere).".
    until false {
        wait PASSIVE_POLL.
        if not otherMcHolderExists() {
            print "dispatch_listener: no mc holder found; promoting.".
            claimMc().
            runActive().
        }
    }
}

// Beats on real time, not UT: under rails warp one physics tick can span
// thousands of game seconds, and a UT schedule would fire on every tick.
when hbPath <> "" and kuniverse:realtime > hbNext then {
    sendHeartbeat().
    set hbNext to kuniverse:realtime + HEARTBEAT_PERIOD.
    return true.
}

if otherMcHolderExists() {
    runPassive().
} else {
    claimMc().
    runActive().
}
