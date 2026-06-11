import { Route, Router as SolidRouter } from "@solidjs/router";
import { lazy } from "solid-js";
import Layout from "~/app/Layout";
import NotesRoute from "~/modules/notes/routes/NotesRoute";

const AskRoute = lazy(() => import("~/modules/qa/routes/AskRoute"));
const SettingsRoute = lazy(
  () => import("~/modules/settings/routes/SettingsRoute")
);

export function Router() {
  return (
    <SolidRouter>
      <Route path="/" component={Layout}>
        <Route path="/" component={NotesRoute} />
        <Route path="/notes" component={NotesRoute} />
        <Route path="/ask" component={AskRoute} />
        <Route path="/settings" component={SettingsRoute} />
      </Route>
    </SolidRouter>
  );
}
