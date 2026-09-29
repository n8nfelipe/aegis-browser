(function () {
  "use strict";

  const extensionMarker = "data-aegis-privacy-guard";
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
      if (parameter.toLowerCase().startsWith("utm_") || trackingParameters.has(parameter.toLowerCase())) {
        url.searchParams.delete(parameter);
        changed = true;
      }
    }

    return changed ? url.href : null;
  }

  function cleanLink(link) {
    const cleaned = cleanUrl(link.href);
    if (cleaned) {
      link.href = cleaned;
      link.dataset.aegisCleaned = "true";
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

  function isLikelyOverlay(element) {
    if (!(element instanceof HTMLElement)) {
      return false;
    }

    const style = window.getComputedStyle(element);
    const bounds = element.getBoundingClientRect();
    const viewportArea = window.innerWidth * window.innerHeight;
    const elementArea = Math.max(0, bounds.width) * Math.max(0, bounds.height);
    const text = `${element.id} ${element.className} ${element.getAttribute("aria-label") || ""}`.toLowerCase();
    const hasConsentLanguage = /cookie|consent|privacy|lgpd|gdpr|newsletter|subscribe|assine|inscri/.test(text);
    const isPositioned = style.position === "fixed" || style.position === "sticky";
    const isLarge = viewportArea > 0 && elementArea / viewportArea > 0.08;
    const isLayer = Number.parseInt(style.zIndex || "0", 10) >= 10;

    return hasConsentLanguage && isPositioned && (isLarge || isLayer);
  }

  function removeInvasiveOverlay(element) {
    if (!isLikelyOverlay(element) || element.hasAttribute(extensionMarker)) {
      return false;
    }

    element.setAttribute(extensionMarker, "overlay");
    element.classList.add("aegis-privacy-hidden");
    return true;
  }

  function cleanOverlays(root) {
    if (!(root instanceof Element || root instanceof Document)) {
      return;
    }

    const candidates = root instanceof HTMLElement && isLikelyOverlay(root)
      ? [root, ...root.querySelectorAll("*")]
      : root.querySelectorAll(root instanceof Document ? "body *" : "*");

    let removed = 0;
    for (const candidate of candidates) {
      if (removeInvasiveOverlay(candidate)) {
        removed += 1;
      }
    }

    if (removed > 0) {
      showStatus(`${removed} aviso${removed === 1 ? "" : "s"} ocultado${removed === 1 ? "" : "s"}`);
    }
  }

  function showStatus(message) {
    if (!document.body || document.querySelector(".aegis-privacy-status")) {
      return;
    }

    const status = document.createElement("div");
    status.className = "aegis-privacy-status";
    status.setAttribute("role", "status");
    status.innerHTML = `<span>Aegis Privacy Guard</span><small>${message}</small><button type="button" aria-label="Fechar aviso">×</button>`;
    status.querySelector("button").addEventListener("click", () => status.remove());
    document.body.appendChild(status);
    window.setTimeout(() => status.remove(), 5000);
  }

  function start() {
    if (!document.documentElement) {
      return;
    }

    document.documentElement.setAttribute(extensionMarker, "active");
    cleanLinks(document);
    cleanOverlays(document);

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
            cleanLinks(node);
            cleanOverlays(node);
          }
        }
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
