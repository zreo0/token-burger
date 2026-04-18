import { HashRouter, Routes, Route } from 'react-router-dom';
import Popup from './pages/Popup';
import Settings from './pages/Settings';

function App() {
    return (
        <HashRouter>
            <Routes>
                <Route path="/" element={<Popup />} />
                <Route path="/settings" element={<Settings />} />
            </Routes>
        </HashRouter>
    );
}

export default App;
