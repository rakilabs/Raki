import "~/shared/styles/index.css";
import { render } from "solid-js/web";
import { App } from "~/app/App";

render(() => <App />, document.getElementById("root") as HTMLElement);

// Hide splash screen once Solid has mounted
const splash = document.getElementById("splash");
if (splash) {
  splash.classList.add("hidden");
  setTimeout(() => splash.remove(), 350);
}
