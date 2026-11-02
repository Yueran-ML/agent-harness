import type { ReactNode } from 'react';
import { useWords } from '../app/locale';
import type { Monitor } from './engine';
import { Transport } from './Transport';
import { Viewer, type ViewerNotice } from './Viewer';
import type { ViewBridge } from './useViewerRect';
import type { MonitorControls } from './useMonitor';

/**
 * A monitor: its name and what it has loaded, the engine's view of it, and its own transport. A
 * press or the keyboard anywhere in it makes it the monitor that the keys go to, as a click on a
 * viewer does in DaVinci Resolve. Hidden, it keeps its transport and its place in the engine, and
 * its view is hidden.
 */
export function MonitorPane({ controls, bridge, native, active, notice, onRetry, hidden = false, focused = false, onFocus, name, hint, switcher, tools, footer }: {
  controls: MonitorControls; bridge: ViewBridge; native: boolean; active: boolean; notice: ViewerNotice | null; onRetry: () => void;
  hidden?: boolean; focused?: boolean; onFocus?: (monitor: Monitor) => void;
  /** The name of what is loaded. */
  name?: string;
  /** What is said under the viewer while nothing is loaded. */
  hint?: string;
  /** In place of the name of the monitor: the switch between the two, where only one is shown. */
  switcher?: ReactNode;
  /** The controls at the right of the title bar: the timeline monitor's Bypass Color on the Cut and Edit pages. */
  tools?: ReactNode;
  /** What is said beneath the picture, above the transport: the Color page's missing LUTs. */
  footer?: ReactNode;
}) {
  const words = useWords().label.studio.monitor;
  const { monitor } = controls;
  const focus = () => onFocus?.(monitor);
  return <section className="monitor" aria-label={monitor === 'source' ? words.sourceTitle : words.timelineTitle} data-monitor={monitor}
    data-focused={onFocus ? focused : undefined} hidden={hidden} onPointerDownCapture={focus} onFocusCapture={focus}>
    {(onFocus || switcher) && <header className="monitor-head">
      {switcher ?? <span className="monitor-name">{words[monitor]}</span>}
      {name && <strong className="monitor-loaded" title={name}>{name}</strong>}
      {tools && <span className="monitor-tools">{tools}</span>}
    </header>}
    <Viewer bridge={bridge} native={native} monitor={monitor} active={active && !hidden} notice={notice} onRetry={onRetry} />
    {hint && !controls.loaded && (notice === null || notice.kind === 'browser') && <p className="caption monitor-hint" data-test="load-hint">{hint}</p>}
    {footer}
    {/* A change of the colour management is made apart from the monitor, which shows the colour before it meanwhile (PROTOCOL.md, "Grades in the picture"). */}
    {controls.state?.monitor === 'timeline' && controls.state.color_error
      ? <p className="caption monitor-colour" role="alert" data-test="colour-failed">{words.colorFailed(controls.state.color_error)}</p>
      : controls.state?.monitor === 'timeline' && controls.state.color_pending
        && <p className="caption monitor-colour" role="status" aria-busy="true" data-test="colour-pending">{words.colorPending}</p>}
    <Transport state={controls.state} rate={controls.rate} last={controls.last} frame={controls.frame} held={controls.held} onCommand={controls.run}
      onSeek={controls.seek} onDrag={controls.drag} onProxies={controls.proxies} />
  </section>;
}
