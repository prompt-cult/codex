chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "threadbox/runtime/ping") {
    sendResponse({ ok: true, runtime: "browser-wasm" });
    return false;
  }

  return false;
});
