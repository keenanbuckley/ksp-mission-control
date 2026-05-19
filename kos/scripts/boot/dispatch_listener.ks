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

local MC_TAG       is "mc".
local PASSIVE_POLL is 5.   // seconds between vessel-change re-checks

local SCRIPT_ARITY is lexicon(
    "launch.ks", 6,
    "maneuver.ks", 0
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
    if op = "toggle_ag" {
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
        if not content:haskey("args") {
            print "dispatch_listener: run_script missing args; dropping.".
            return.
        }
        local p is content:path.
        local a is content:args.
        if not a:istype("List") {
            print "dispatch_listener: run_script args must be a list; dropping.".
            return.
        }
        if not SCRIPT_ARITY:haskey(p) {
            print "dispatch_listener: unknown script: /" + p + "; dropping.".
            return.
        }
        local expected is SCRIPT_ARITY[p].
        if a:length <> expected {
            print "dispatch_listener: /" + p + " expects " + expected + " args; got " + a:length + "; dropping.".
            return.
        }
        if not exists("/" + p) {
            print "dispatch_listener: script not found: /" + p + "; dropping.".
            return.
        }
        ackOp(op).
        print "dispatch_listener: running /" + p + ".".
        if expected = 0 {
            runPath("/" + p).
        } else if expected = 6 {
            runPath("/" + p, a[0], a[1], a[2], a[3], a[4], a[5]).
        } else {
            print "dispatch_listener: arity " + expected + " has no runPath form; dropping.".
            return.
        }
        print "dispatch_listener: /" + p + " returned.".
        // kerboscript has no try/catch, so a script that aborts mid-flight
        // never reaches this line and the server sees no script_done event.
        sendEvent(lexicon("kind", "script_done", "path", p, "ok", true)).
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

if otherMcHolderExists() {
    runPassive().
} else {
    claimMc().
    runActive().
}
