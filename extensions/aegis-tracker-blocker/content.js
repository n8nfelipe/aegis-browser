(function () {
  "use strict";

  const marker = "data-aegis-tracker-blocked";
  const statusClass = "aegis-tracker-status";

  // The list is intentionally local and auditable. It does not download a
  // remote list or send browsing data anywhere.
  const blockedHostSuffixes = [
    "adform.net",
    "adnxs.com",
    "adsrvr.org",
    "amplitude.com",
    "analytics.tiktok.com",
    "branch.io",
    "clarity.ms",
    "clicky.com",
    "connect.facebook.net",
    "criteo.com",
    "demdex.net",
    "doubleclick.net",
    "facebook.net",
    "fullstory.com",
    "googlesyndication.com",
    "google-analytics.com",
    "googleadservices.com",
    "googletagmanager.com",
    "googletagservices.com",
    "heap.io",
    "hotjar.com",
    "hubspot.com",
    "matomo.cloud",
    "mixpanel.com",
    "moatads.com",
    "mouseflow.com",
    "newrelic.com",
    "nr-data.net",
    "omtrdc.net",
    "openx.net",
    "outbrain.com",
    "pippio.com",
    "pixel.facebook.com",
    "pubmatic.com",
    "quantserve.com",
    "scorecardresearch.com",
    "segment.io",
    "smartlook.com",
    "taboola.com",
    "tiktok.com",
    "triplelift.com",
    "twitter.com",
    "unrulymedia.com",
    "yieldmo.com"
  ];

  const trackingParameters = new Set([
    "_ga",
    "_gl",
    "dclid",
    "fbclid",
    "gclid",
    "igshid",
    "mc_cid",
    "mc_eid",
    "msclkid",
    "oly_anon_id",
    "oly_enc_id",
    "ref_src",
    "s_cid",
    "vero_conv",
    "vero_id"
  ]);

  const blockedTags = new Set([
    "EMBED",
    "IFRAME",
    "IMG",
    "LINK",
    "OBJECT",
    "SCRIPT",
    "SOURCE",
    "TRACK",
    "VIDEO"
  ]);

  const counters = {
    blocked: 0,
    cleaned: 0
  };

  function isHttpUrl(rawUrl) {
    try {
      const url = new URL(rawUrl, window.location.href);
      return url.protocol === "http:" || url.protocol === "https:";
    } catch (_) {
      return false;
    }
  }

  function isBlockedHost(hostname) {
    const normalized = hostname.toLowerCase().replace(/\.$/, "");
    return blockedHostSuffixes.some((suffix) => (
      normalized === suffix || normalized.endsWith(`.${suffix}`)
    ));
  }

  function isTrackerUrl(rawUrl) {
    if (!rawUrl || !isHttpUrl(rawUrl)) {
      return false;
    }

    try {
      return isBlockedHost(new URL(rawUrl, window.location.href).hostname);
    } catch (_) {
      return false;
    }
  }

  function resourceUrl(element) {
    for (const attribute of ["src", "href", "data", "data-src", "data-url", "data-href"]) {
      const value = element.getAttribute(attribute);
      if (value) {
        return value;
      }
    }
    return "";
  }

  function blockResource(element) {
    if (!(element instanceof Element) || !blockedTags.has(element.tagName)) {
      return false;
    }

    if (element.hasAttribute(marker)) {
      return false;
    }

    const url = resourceUrl(element);
    if (!isTrackerUrl(url)) {
      return false;
    }

    element.setAttribute(marker, "resource");
    element.remove();
    counters.blocked += 1;
    return true;
  }

  function blockResources(root) {
    if (!(root instanceof Element || root instanceof Document)) {
      return;
    }

    if (root instanceof Element) {
      blockResource(root);
    }

    const selector = [
      "embed[src]",
      "iframe[src]",
      "img[src]",
      "link[href]",
      "object[data]",
      "script[src]",
      "source[src]",
      "track[src]",
      "video[src]"
    ].join(",");

    for (const element of root.querySelectorAll(selector)) {
      blockResource(element);
    }
  }

  function cleanUrl(rawUrl) {
    let url;
    try {
      url = new URL(rawUrl, window.location.href);
    } catch (_) {
      return null;
    }

    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return null;
    }

    let changed = false;
    for (const parameter of [...url.searchParams.keys()]) {
      const normalized = parameter.toLowerCase();
      if (normalized.startsWith("utm_") || trackingParameters.has(normalized)) {
        url.searchParams.delete(parameter);
        changed = true;
      }
    }

    return changed ? url.href : null;
  }

  function cleanLink(link) {
    if (!(link instanceof HTMLAnchorElement)) {
      return;
    }

    const cleaned = cleanUrl(link.href);
    if (cleaned) {
      link.href = cleaned;
      link.setAttribute("data-aegis-tracker-cleaned", "true");
      counters.cleaned += 1;
    }
  }

  function cleanLinks(root) {
    if (!(root instanceof Element || root instanceof Document)) {
      return;
    }

    if (root instanceof HTMLAnchorElement) {
      cleanLink(root);
    }

    for (const link of root.querySelectorAll("a[href]")) {
      cleanLink(link);
    }
  }

  function showStatus() {
    if (window.top !== window || !document.body || document.querySelector(`.${statusClass}`)) {
      return;
    }

    const status = document.createElement("div");
    status.className = statusClass;
    status.setAttribute("role", "status");
    status.innerHTML = `<span>Aegis Tracker Blocker</span><small>${counters.blocked} recurso${counters.blocked === 1 ? "" : "s"} bloqueado${counters.blocked === 1 ? "" : "s"}</small><button type="button" aria-label="Fechar aviso">×</button>`;
    status.querySelector("button").addEventListener("click", () => status.remove());
    document.body.appendChild(status);
    window.setTimeout(() => status.remove(), 5000);
  }

  function start() {
    if (!document.documentElement) {
      return;
    }

    document.documentElement.setAttribute("data-aegis-tracker-blocker", "active");
    blockResources(document);
    cleanLinks(document);

    document.addEventListener("click", (event) => {
      const link = event.target instanceof Element ? event.target.closest("a[href]") : null;
      if (link) {
        cleanLink(link);
      }
    }, true);

    const observer = new MutationObserver((mutations) => {
      for (const mutation of mutations) {
        for (const node of mutation.addedNodes) {
          if (node instanceof Element) {
            blockResources(node);
            cleanLinks(node);
          }
        }
      }
      if (counters.blocked > 0) {
        showStatus();
      }
    });
    observer.observe(document.documentElement, { childList: true, subtree: true });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
})();
