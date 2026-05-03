import DefaultTheme from 'vitepress/theme'
import './custom.css'

import Diagram from './components/Diagram.vue'
import Landing from './components/Landing.vue'
import KymaArchitectureDiagram from './components/diagrams/KymaArchitectureDiagram.vue'
import KymaPruningCascade from './components/diagrams/KymaPruningCascade.vue'
import KymaMultiSourceFlow from './components/diagrams/KymaMultiSourceFlow.vue'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('Diagram', Diagram)
    app.component('Landing', Landing)
    app.component('KymaArchitectureDiagram', KymaArchitectureDiagram)
    app.component('KymaPruningCascade', KymaPruningCascade)
    app.component('KymaMultiSourceFlow', KymaMultiSourceFlow)
  },
}
