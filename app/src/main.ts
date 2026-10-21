import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { initPlatform } from "./lib/platform";
import { applyStoredTheme } from "./lib/theme";

applyStoredTheme();
await initPlatform();

export default mount(App, { target: document.getElementById("app")! });
