import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App, NoData } from './App';
import { readInlineData } from './data/inline';
import { parseReportPayload, reportSource } from './data/report';
import { apiSource } from './data/source';
import './styles.css';

// The bench UI: served by `slidock-bench ui` (reading its API), or built as a single-file report whose data is
// inlined in the page.
function start() {
  const inline = readInlineData(document);
  switch (inline.kind) {
    case 'none':
      return <App source={apiSource()} />;
    case 'empty':
      return (
        <NoData title="This report holds no data">
          It is the report template: <span className="font-mono">slidock-bench report</span> fills it.
        </NoData>
      );
    case 'invalid':
      return <NoData title="This report's data is not valid JSON">The file may be truncated.</NoData>;
    case 'data':
      try {
        return <App source={reportSource(parseReportPayload(inline.value))} />;
      } catch (error) {
        return <NoData title="This report's data cannot be read">{String(error)}</NoData>;
      }
  }
}

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(<StrictMode>{start()}</StrictMode>);
}
