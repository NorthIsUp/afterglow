# Rotating on a timer

`SAVER_ROTATE_SECS` moves the panel to another saver every N seconds. **0 is
the default and means off**, so a deployment that does not set it behaves as it
always did; anything outside 0..86400 falls back to 0 rather than being clamped,
which is what every other numeric knob here does.

Like `SAVER`, the env var is only the STARTUP value: the mirror page has an
`auto-rotate` tick box and a minutes field, and `POST /rotate?mins=N` does the
same thing by hand (`mins=0` is off, the ceiling is 1440 — a day, same as the
env var). A value that is not a whole number of minutes in range is a 400 that
changes nothing, because a lenient parse of `5x` would turn rotation off, which
is the one outcome nobody asked for. The change takes effect on the next frame,
with no restart, and `/meta` reports the live interval as `rotate_secs` so a
second browser shows what the first one set rather than its own guess. Minutes
on the wire because that is what a person asks for, seconds in `/meta` because
that is the renderer's unit and the env var's; an interval that is not a whole
number of minutes (only reachable from the env var) shows rounded on the page.

Setting the interval **restarts the turn**, even when the number did not change:
asking for five minutes 4:59 into a five-minute turn must buy five minutes, not
one second. What the render loop reads per frame is one relaxed atomic load of a
control word — the interval in its low half, a change counter in its high half,
which is what makes re-asking for the same number count as a change. No lock, no
env lookup, no clock read: see [CLAUDE.md](../CLAUDE.md) on the frame loop.

`SAVER` still picks the STARTING saver — rotation moves on from there. The order
is a **shuffled bag**: every saver, in random order, none of them again until all
of them have been shown, then reshuffled. That is what "rotate through all the
savers" has to mean — rolling an independent choice each time takes about 95
turns to show you all 25 (coupon collector), eight hours at a five-minute
interval, where the bag takes exactly 25 and two hours.

A bag is not a walk down the table: a walk is predictable in the wrong way (the
same saver always follows the same saver, and the three toaster variants are
adjacent, so a walk shows them back to back to back), and a bag is reshuffled
every cycle. "It never shows the same saver twice in a row" stays a property of
the code rather than a probability — inside a bag the entries are distinct, and
at the boundary between two bags the refill swaps the top entry away if it is
the saver still on screen, rather than re-shuffling until it looks right. The bag
is a fixed-size array sized from the table, shuffled in place, so nothing
allocates. Every saver gets the
same length turn; there is no per-saver table of seconds, because the expensive
ones hold the target fps on this panel and so there is nothing to compensate for.

**Who is in rotation** is a tick box per saver in the mirror page's list (and
per group, on its heading), or `POST /rotation?saver=<name>&on=0|1` /
`POST /rotation?group=<name>&on=0|1` by hand; `/meta` reports the savers out as
`excluded`. A scene goes in or out with its `-wide` twin, because the page shows
the pair as one row, and the pair is one turn in the bag: rotation shows the half
its `expanded` choice names: the `-wide` for a halftone scene and the original
for a text piece, until a click picks the other half (the `expanded` toggle, or
`/select` of either name), which then sticks. Rotation moving on, and the startup
`SAVER`, do not change the choice. The default is one function,
`saver::expanded_by_default`. `SAVER_ROTATE_EXCLUDE` (comma-separated names, default none)
is the startup set, so a deployment can pin it; like the interval, the page moves
it live and a restart goes back to the env value. An unknown name there is
logged and ignored.

A saver out of rotation is skipped when it comes out of the bag rather than taken
out of it, so a change applies from the next turn, the bag still covers every
saver that is in once per cycle, and the refill stays the same allocation-free
shuffle. Taking out the saver on screen does not switch away; it is just not
picked again. Taking out every saver pauses rotation — the turn comes up and
nothing moves, and the page says so — and a click still shows any saver. The set
is a bit per saver, read only when a turn is up, never per frame.

Clicking a saver on the mirror page **restarts the interval**, so a manual pick
always gets a whole turn rather than the two seconds that happened to be left.
It does not pause rotation: a pause needs a resume, which is a second knob plus
a page that has to show which mode it is in, to save someone setting this to 0.

A rotation is the same event as a click from `Driver::switch` down, including the
epoch bump — so every connected viewer's stream ends, it re-reads `/meta` and
takes a keyframe. That is one keyframe per viewer per interval (32 KB for
`matrix`, 1 MB for `blocks`, which is the widest grid here), on the viewer's own
thread, and it is the cost a click has always had. The page used to sit out its
two-second reconnect backoff and show an error banner on a stream that ended
without a click; it now reconnects immediately and silently, because with this
knob on that is a routine event rather than a fault.
