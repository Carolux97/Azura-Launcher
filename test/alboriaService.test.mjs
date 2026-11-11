/*
 * Tests for Alboria's notices and the service's state in the launcher.
 *
 * Run with: npm test
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { formatNotice, minutesUntil, noticeKey, olderThan, serviceView } from "../src/alboriaService.js";

test("a notice keeps every formatting tag", () => {
  const html = '<h3>Hoy</h3><p><b>a</b> <i>b</i> <u>c</u> <s>d</s> <mark>e</mark> <code>f</code></p><ul><li>g</li></ul><blockquote>h</blockquote><hr>';
  assert.equal(formatNotice(html), html);
  assert.equal(formatNotice("uno<br>dos<br/>tres"), "uno<br>dos<br>tres");
});

test("a notice keeps https and mailto links and drops any other", () => {
  assert.equal(formatNotice('<a href="https://alboria.org">web</a>'), '<a href="https://alboria.org">web</a>');
  assert.equal(formatNotice("<a href='mailto:hola@alboria.org'>m</a>"), '<a href="mailto:hola@alboria.org">m</a>');
  assert.equal(formatNotice('<a href="javascript:alert(1)">x</a>'), "<a>x</a>");
  assert.equal(formatNotice('<a href="http://alboria.org">x</a>'), "<a>x</a>");
  assert.equal(formatNotice('<a href="https&#58;//x">x</a>'), "<a>x</a>");
});

test("a notice keeps a text colour and no other style", () => {
  assert.equal(formatNotice('<span style="color: #d9306b">rojo</span>'), '<span style="color: #d9306b">rojo</span>');
  assert.equal(formatNotice('<span style="color: rgb(217, 48, 107);">x</span>'), '<span style="color: rgb(217, 48, 107);">x</span>');
  assert.equal(formatNotice('<span style="background:url(https://x.example/t.png)">x</span>'), "<span>x</span>");
  assert.equal(formatNotice('<span style="color:red;background:url(https://x.example/t.png)">x</span>'), "<span>x</span>");
});

test("a notice loses whatever could run code or load files, with what is inside", () => {
  const html =
    '<img src=x onerror=alert(1)><script>alert(2)</script><svg onload=alert(3)><circle/></svg>' +
    '<iframe src="https://x.example"></iframe><style>p{}</style><p onclick="alert(4)">ok</p><!-- <b>no</b> -->';
  assert.equal(formatNotice(html), "<p>ok</p>");
  assert.equal(formatNotice("<SCRIPT>alert(1)</SCRIPT>after"), "after");
  assert.equal(formatNotice("<script>never closed"), "");
});

test("a notice keeps the text of unknown tags, escapes plain text and closes what is left open", () => {
  assert.equal(formatNotice("<blink>hola</blink>"), "hola");
  assert.equal(formatNotice("2 < 3 & 4 > 1"), "2 &lt; 3 &amp; 4 &gt; 1");
  assert.equal(formatNotice("Tom &amp; Jerry &copy;"), "Tom &amp; Jerry &copy;");
  assert.equal(formatNotice("<b>sin cerrar"), "<b>sin cerrar</b>");
  assert.equal(formatNotice("<b><i>cruzado</b></i>"), "<b><i>cruzado</i></b>");
  assert.equal(formatNotice('"comillas"'), "&quot;comillas&quot;");
  assert.equal(formatNotice(null), "");
});

test("versions compare as the server compares them", () => {
  assert.equal(olderThan("0.5.1", "0.6.0"), true);
  assert.equal(olderThan("0.6.0", "0.6.0"), false);
  assert.equal(olderThan("0.10.0", "0.9.9"), false);
  assert.equal(olderThan("0.6.0", ""), false);
});

const NOW = Date.parse("2026-11-01T12:00:00Z");
const base = { emailCodes: true, maintenance: false, maintenanceUntil: null, motd: null, game: null, warnings: [] };

test("nothing to show while the service is fine", () => {
  assert.deepEqual(serviceView(base, "0.6.0", NOW), { block: null, notices: [], warnings: [] });
});

test("an unreachable service is its own state", () => {
  assert.deepEqual(serviceView(null, "0.6.0", NOW).block, { kind: "down" });
});

test("what keeps a player out of new rooms comes in the server's order", () => {
  const game = { online: false, maintenance: true, maintenanceUntil: null, motd: null, minLauncher: "0.6.0" };
  assert.deepEqual(serviceView({ ...base, maintenance: true, game }, "0.5.1", NOW).block, { kind: "outdated", min: "0.6.0" });
  assert.deepEqual(serviceView({ ...base, maintenance: true, game }, "0.6.0", NOW).block, { kind: "gameOff" });
  const until = "2026-11-01T18:00:00Z";
  assert.deepEqual(serviceView({ ...base, maintenance: true, maintenanceUntil: until, game: { ...game, online: true } }, "0.6.0", NOW).block, {
    kind: "maintenance",
    scope: "all",
    until: Date.parse(until),
  });
  assert.deepEqual(serviceView({ ...base, game: { ...game, online: true } }, "0.6.0", NOW).block, { kind: "maintenance", scope: "game", until: null });
});

test("maintenance that already ran out is over", () => {
  const view = serviceView({ ...base, maintenance: true, maintenanceUntil: "2026-11-01T11:00:00Z" }, "0.6.0", NOW);
  assert.equal(view.block, null);
});

test("notices come made safe, the one for everyone first", () => {
  const view = serviceView({ ...base, motd: "<b>Hola</b><script>x</script>", game: { online: true, motd: "Solo Gears" } }, "0.6.0", NOW);
  assert.deepEqual(view.notices, [
    { scope: "all", html: "<b>Hola</b>" },
    { scope: "game", html: "Solo Gears" },
  ]);
  assert.notEqual(noticeKey(view.notices[0]), noticeKey(view.notices[1]));
});

test("countdowns come soonest first, only for the actions the launcher warns about", () => {
  const warnings = [
    { action: "restart", scope: "all", at: "2026-11-01T12:30:00Z" },
    { action: "close_rooms", scope: "game", at: "2026-11-01T12:05:00Z" },
    { action: "maint_off", scope: "all", at: "2026-11-01T12:10:00Z" },
    { action: "restart", scope: "all", at: "2026-11-01T11:59:00Z" },
  ];
  const view = serviceView({ ...base, warnings }, "0.6.0", NOW);
  assert.deepEqual(
    view.warnings.map((w) => [w.key, w.thenKey]),
    [["warnCloseRooms", "thenCloseRooms"], ["warnRestart", "thenRestart"]],
  );
});

test("no countdown for maintenance that is already on", () => {
  const warnings = [{ action: "maint_on", scope: "all", at: "2026-11-01T12:30:00Z" }];
  assert.equal(serviceView({ ...base, maintenance: true, warnings }, "0.6.0", NOW).warnings.length, 0);
  assert.equal(serviceView({ ...base, warnings }, "0.6.0", NOW).warnings.length, 1);
});

test("minutes until an action are whole and at least one", () => {
  assert.equal(minutesUntil(NOW + 12 * 60_000 - 1000, NOW), 12);
  assert.equal(minutesUntil(NOW + 10_000, NOW), 1);
});
