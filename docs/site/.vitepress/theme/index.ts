import DefaultTheme from 'vitepress/theme'
import './custom.css'

import Diagram from './components/Diagram.vue'
import Landing from './components/Landing.vue'
import PensieveArchitectureDiagram from './components/diagrams/PensieveArchitectureDiagram.vue'
import PensievePruningCascade from './components/diagrams/PensievePruningCascade.vue'
import PensieveMultiSourceFlow from './components/diagrams/PensieveMultiSourceFlow.vue'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('Diagram', Diagram)
    app.component('Landing', Landing)
    app.component('PensieveArchitectureDiagram', PensieveArchitectureDiagram)
    app.component('PensievePruningCascade', PensievePruningCascade)
    app.component('PensieveMultiSourceFlow', PensieveMultiSourceFlow)
  },
}
