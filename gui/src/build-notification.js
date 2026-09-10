export function notifyBuildComplete(success, title, notificationApi = globalThis.Notification) {
  if (notificationApi.permission === "granted") {
    new notificationApi(success ? "Build Complete" : "Build Failed", {
      body: success ? `"${title}" built successfully` : `"${title}" build failed`,
    });
  } else if (notificationApi.permission !== "denied") {
    notificationApi.requestPermission();
  }
}
