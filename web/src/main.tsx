import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRootRoute, createRoute, createRouter, Outlet, RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { ApiError } from "./api";
import { AuthPage } from "./pages/AuthPage";
import { LinksPage } from "./pages/LinksPage";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: {
    // 401 and 404 will not fix themselves; retrying them only delays the right screen.
    queries: {
      retry: (n, err) => n < 2 && !(err instanceof ApiError && err.status >= 400 && err.status < 500),
    },
  },
});

const authSearch = (s: Record<string, unknown>) => ({
  email: typeof s.email === "string" ? s.email : undefined,
  expired: s.expired === true || s.expired === "1" ? true : undefined,
});

const root = createRootRoute({ component: Outlet });
const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/", component: LinksPage }),
  createRoute({
    getParentRoute: () => root,
    path: "/login",
    validateSearch: authSearch,
    component: () => <AuthPage mode="login" />,
  }),
  createRoute({
    getParentRoute: () => root,
    path: "/register",
    validateSearch: authSearch,
    component: () => <AuthPage mode="register" />,
  }),
]);
const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

const el = document.getElementById("root");
if (!el) throw new Error("#root missing from index.html");
createRoot(el).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
