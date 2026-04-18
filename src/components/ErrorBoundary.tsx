import { Component, type ErrorInfo, type ReactNode } from 'react';

interface Props {
    children: ReactNode;
    fallback?: ReactNode;
}

interface State {
    hasError: boolean;
    error: Error | null;
}

class ErrorBoundary extends Component<Props, State> {
    constructor(props: Props) {
        super(props);
        this.state = { hasError: false, error: null };
    }

    static getDerivedStateFromError(error: Error): State {
        return { hasError: true, error };
    }

    componentDidCatch(error: Error, info: ErrorInfo) {
        console.error('[ErrorBoundary]', error, info.componentStack);
    }

    render() {
        if (this.state.hasError) {
            if (this.props.fallback) {
                return this.props.fallback;
            }
            return (
                <div style={{ padding: 16, textAlign: 'center', color: '#999' }}>
                    <p>Something went wrong.</p>
                    <button
                        onClick={() => this.setState({ hasError: false, error: null })}
                        style={{ marginTop: 8, cursor: 'pointer' }}
                    >
                        Retry
                    </button>
                </div>
            );
        }
        return this.props.children;
    }
}

export default ErrorBoundary;
