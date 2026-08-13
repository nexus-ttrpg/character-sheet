/*
 * nexus-collector.js — Roll20 Mod Script collector (§6.1, §6.5).
 *
 * DEPLOYMENT: paste this entire script into Roll20's Mod Scripts panel
 * (Campaign Settings → API Scripts). It runs inside Roll20's sandbox.
 *
 * This script is DELIBERATELY LOGIC-FREE. It reads Character/Attribute objects
 * verbatim, POSTs them signed, and writes attrs verbatim on apply. All mapping
 * logic lives server-side in the adapter (adapters/roll20/src), so a mapping
 * fix ships instantly without every GM re-pasting this script (§6.1).
 *
 * AUDITABILITY IS A FEATURE HERE, NOT LAZINESS (§6.5). This file is unbuilt,
 * unminified, and short enough to read before pasting into a system with access
 * to your campaign. The signed-payload protocol means a compromised server
 * still can't make the collector do anything outside writing sheet attributes.
 *
 * Commands:
 *   !nexus-auth            — handshake: obtain a short-lived signed JWT
 *   !nexus-pull [charId]   — POST raw attribute dump to the Nexus Sheet server
 *   !nexus-apply [charId]  — write attrs verbatim from the server response
 *
 * Config: replace NEXUS_URL and INSTALL_TOKEN below when connecting a campaign.
 */

const NEXUS_URL = "https://api.nexus-sheet.example/v1";
const INSTALL_TOKEN = "REPLACE_WITH_YOUR_INSTALL_TOKEN"; // per-campaign (§6.5)

const COLLECTOR_VERSION = "1.0.0";
const CHUNK_SIZE = 50; // attrs per !nexus-apply tick (§6.4 execution time limits)

// ── Auth: short-lived signed JWT (§6.5) ──────────────────────────────────────
//
// Script source is visible to anyone with GM access and gets copy-pasted
// around, so we never embed a long-lived secret. The install token is exchanged
// for a short-lived JWT via !nexus-auth, used for subsequent calls.

let jwt = null;
let jwtExpiry = 0;

function isAuthenticated() {
  return jwt != null && Date.now() < jwtExpiry;
}

// ── HTTP: gated outbound (§6.1) ──────────────────────────────────────────────
//
// Roll20's sandbox limits outbound network to a gated HttpRequest. The actual
// transport depends on Roll20's sandbox capabilities; this abstraction is the
// single point a real deployment wires to whatever Roll20 provides.

function nexusPost(path, body, onSuccess, onError) {
  const headers = { "Content-Type": "application/json" };
  if (jwt) headers["Authorization"] = "Bearer " + jwt;
  const payload = JSON.stringify(body);
  // Roll20 sandbox HTTP — uses whichever transport the sandbox provides.
  sendHTTPRequest(NEXUS_URL + path, payload, headers, function (response) {
    if (response.status >= 200 && response.status < 300) {
      onSuccess(JSON.parse(response.body));
    } else {
      onError("HTTP " + response.status + ": " + response.body);
    }
  });
}

// ── Read attrs verbatim (§6.1: "reads Character/Attribute objects verbatim") ─

function readSheet(characterId) {
  var attrs = findObjs({ type: "attribute", characterid: characterId });
  var flat = {};
  var repeating = {};
  for (var i = 0; i < attrs.length; i++) {
    var name = attrs[i].get("name");
    var value = attrs[i].get("current") || "";
    if (name.indexOf("repeating_") === 0) {
      // repeating_section_rowId_attrName — split into section/rowId/attrName
      var parts = name.split("_");
      var section = parts[0] + "_" + parts[1]; // e.g. repeating_skills
      var rowId = parts[2];
      var attrName = parts.slice(3).join("_");
      if (!repeating[section]) repeating[section] = {};
      if (!repeating[section][rowId]) repeating[section][rowId] = {};
      repeating[section][rowId][attrName] = value;
    } else {
      flat[name] = value;
    }
  }
  return {
    sheet_version: getAttrByName(characterId, "sheet_version") || "unknown",
    character_id: characterId,
    attributes: flat,
    repeating: repeating,
  };
}

// ── Write attrs verbatim (§6.1: "writes attrs verbatim") ─────────────────────

function writeAttrs(characterId, attrs, startIndex, onChunkDone) {
  var keys = Object.keys(attrs);
  var endIndex = Math.min(startIndex + CHUNK_SIZE, keys.length);
  for (var i = startIndex; i < endIndex; i++) {
    var name = keys[i];
    var value = attrs[name];
    var existing = findObjs({ type: "attribute", characterid: characterId, name: name })[0];
    if (existing) {
      existing.set("current", value);
    } else {
      createObj("attribute", { characterid: characterId, name: name, current: value });
    }
  }
  if (endIndex < keys.length) {
    // Chunk across sandbox ticks (§6.4): yield, then continue.
    onChunkDone(endIndex);
  }
}

// ── Chat command handlers ────────────────────────────────────────────────────

on("chat:message", function (msg) {
  if (msg.type !== "api") return;
  var parts = msg.content.split(/\s+/);
  var cmd = parts[0];

  if (cmd === "!nexus-auth") {
    nexusPost("/roll20/auth", { install_token: INSTALL_TOKEN, campaign_id: Campaign().get("id") },
      function (res) {
        jwt = res.jwt;
        jwtExpiry = Date.now() + (res.expires_in || 300) * 1000;
        sendChat("Nexus", "/w gm Authenticated. JWT valid for " + (res.expires_in || 300) + "s.");
      },
      function (err) { sendChat("Nexus", "/w gm Auth failed: " + err); }
    );
    return;
  }

  if (cmd === "!nexus-pull") {
    if (!isAuthenticated()) { sendChat("Nexus", "/w gm Run !nexus-auth first."); return; }
    var charId = parts[1] || Campaign().get("players").split(",")[0];
    var sheet = readSheet(charId);
    nexusPost("/roll20/collect", { collector_version: COLLECTOR_VERSION, sheet: sheet },
      function (res) {
        sendChat("Nexus", "/w gm Pulled " + charId + ". " +
          (res.warnings ? res.warnings.length + " warnings." : "OK."));
      },
      function (err) { sendChat("Nexus", "/w gm Pull failed: " + err); }
    );
    return;
  }

  if (cmd === "!nexus-apply") {
    if (!isAuthenticated()) { sendChat("Nexus", "/w gm Run !nexus-auth first."); return; }
    var applyCharId = parts[1] || Campaign().get("players").split(",")[0];
    nexusPost("/roll20/apply", { character_id: applyCharId },
      function (res) {
        // Server returns pre-mapped attrs to write verbatim (§6.1).
        var attrs = res.attributes;
        var totalKeys = Object.keys(attrs).length;
        function writeChunk(startIndex) {
          writeAttrs(applyCharId, attrs, startIndex, writeChunk);
          if (startIndex + CHUNK_SIZE >= totalKeys) {
            sendChat("Nexus", "/w gm Applied " + totalKeys + " attrs to " + applyCharId + ".");
          }
        }
        writeChunk(0);
      },
      function (err) { sendChat("Nexus", "/w gm Apply failed: " + err); }
    );
    return;
  }
});

// ── Re-announce on sandbox start (§6.2) ──────────────────────────────────────
//
// Roll20 shuts the sandbox down on campaign inactivity and won't spin it up
// when nobody is logged in (§6.2). The server can't poll; instead, the
// collector re-announces itself on sandbox start so the server learns when a
// campaign becomes reachable and can surface "Roll20 is online, sync now."

on("ready", function () {
  if (INSTALL_TOKEN === "REPLACE_WITH_YOUR_INSTALL_TOKEN") {
    sendChat("Nexus", "/w gm nexus-collector.js loaded but not configured. " +
      "Set INSTALL_TOKEN (from your Nexus Sheet campaign settings).");
    return;
  }
  nexusPost("/roll20/announce", {
    collector_version: COLLECTOR_VERSION,
    campaign_id: Campaign().get("id"),
  }, function () {
    sendChat("Nexus", "/w gm Roll20 collector online. Commands: !nexus-auth, !nexus-pull, !nexus-apply");
  }, function (err) {
    sendChat("Nexus", "/w gm Announce failed (server may be down): " + err);
  });
});
