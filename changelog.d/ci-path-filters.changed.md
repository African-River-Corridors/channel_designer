- **CI runs the slow non-required jobs only when their inputs change (contributors).** On a pull request,
  the app, web, bundle, wheel and website jobs run only when the files they build from change; main runs
  them as before. CodeQL can move to a push to main and a weekly run. A failed scheduled run opens an issue.
  The required checks `test` and `release-guard` still run on every pull request.
