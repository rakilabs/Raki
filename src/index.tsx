import "~/shared/styles/index.css";
import { render } from "solid-js/web";
import { App } from "~/app/App";
import { commands } from "~/shared/ipc";

// TEMP: expose typed IPC commands on window for signal-boost testing from the
// dev-tools console. Remove before shipping.
// @ts-expect-error intentional dev-only global
window.__RAKI_COMMANDS__ = commands;

render(() => <App />, document.getElementById("root") as HTMLElement);

// Hide splash screen once Solid has mounted
const splash = document.getElementById("splash");
if (splash) {
  splash.classList.add("hidden");
  setTimeout(() => splash.remove(), 350);
}
