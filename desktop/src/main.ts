import "@stemma-math/design/index.css";
import "@stemma-math/design-elements";
import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";

const target = document.getElementById("app");
if (!target) throw new Error("Missing app root");
mount(App, { target });
