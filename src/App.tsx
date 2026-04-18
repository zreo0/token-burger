import { HashRouter, Routes, Route } from 'react-router-dom';
import { TokenProvider } from './context/TokenContext';
import Popup from './pages/Popup';
import Settings from './pages/Settings';

function App() {
    return (
        <TokenProvider>
            <HashRouter>
                <Routes>
                    <Route path="/" element={<Popup />} />
                    <Route path="/settings" element={<Settings />} />
                </Routes>
            </HashRouter>
        </TokenProvider>
    );
}

export default App;
