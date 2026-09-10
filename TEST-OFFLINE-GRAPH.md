# Offline graph controls

The shared viewport in `src/daemon/html/workflow_view_js.rs` serves live job pages and offline snapshots. The export unit test guards script inclusion and initialization; these browser checks exercise layout, actual scrolling, and navigation.

1. Build with `cargo build`, and use that binary to start an isolated session browser with `SCSH_HOME` and `TMPDIR` pointing into this repository's gitignored `tmp/`. Choose an unused localhost port with `socket.bind(('127.0.0.1', 0))`, then set `SCSH_DAEMON_PORT` to it. Run a host-only workflow with at least six dependent steps, and open its job page. Download **Job snapshot** into `tmp/`, then open the downloaded file directly with `file://`. Keep the live page open for comparison.

   **Predict:** both graphs initially show every node, including Start and Finish, fitted and centered inside the graph viewport. The snapshot works with the browser network disabled.

2. On each page, click the zoom-percentage button to reset to **100%**, then **+** until the graph exceeds its viewport. Drag empty graph space left and right; use horizontal trackpad scrolling. Click **Fit**, then **−** and **+**.

   **Predict:** dragging and horizontal scrolling reach both ends of the graph; Fit restores the complete graph; the zoom percentage changes with the buttons. The surrounding document stays in place. At the fitted minimum, zoom-out is disabled.

3. Open **Full screen**, resize the browser, and click **Fit**. Check at desktop and phone widths (for example 1280 px and 390 px). Close using **Close**, **Escape**, and a click outside the modal. Use Tab and Shift+Tab while expanded.

   **Predict:** Fit uses the available dimensions in either view; the modal stays within the viewport; keyboard focus stays inside it until dismissal. Closing restores ordinary document scrolling. Graph controls remain reachable at narrow widths.

4. Collapse a task row, then click its graph node from the expanded view. Follow a status-summary link as well. Check a snapshot without a workflow graph and one containing a recording.

   **Predict:** graph navigation closes the modal, opens the target row, moves focus to its summary, and updates the task fragment. Other snapshots still load; recording playback and keyboard controls work. The browser console reports no script errors.

5. On the live page, choose a non-default zoom and scroll position, then let the job update. Repeat with a wide graph and a tall graph of many independent steps.

   **Predict:** live updates keep working and preserve the viewer's chosen zoom. The tighter axis determines Fit; vertical panning works for a tall graph just as horizontal panning works for a wide one.

6. Open the downloaded snapshot in Chrome, choose **File → Save Page As…** (Webpage, Complete), and open the saved copy directly. Repeat after saving while the large view was open.

   **Predict:** the copy is a serialization of the live DOM, yet it behaves as the original: the graph opens fitted, zoom and dragging work, the large view opens and closes, and each recorded row holds exactly one player whose ⛶ button and `f` key fill the screen with a centered terminal. Saving while expanded yields a copy that opens collapsed.

Stop the isolated daemon with the same environment and `scsh daemon stop`, close any automated browser, and remove its temporary state. Never stop the developer's default daemon. All checks must pass on both page types; record browser/version and failures when reporting results.
