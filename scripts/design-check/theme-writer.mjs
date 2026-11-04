export const themeWriterRule = {
  meta: { type:'problem',schema:[],messages:{invalid:'R1 主题出口只允许root.style.setProperty(name,value)；禁止其他样式读写。'} },
  create(context) {
    return { MemberExpression(node) {
      if((!node.computed&&node.property.name==='style')||(node.computed&&node.property.value==='style')) {
        const member=node.parent;
        const call=member?.parent;
        if(node.object.type!=='Identifier'||node.object.name!=='root'||member?.type!=='MemberExpression'||member.computed||member.property.name!=='setProperty'||call?.type!=='CallExpression'||call.callee!==member||call.arguments.length!==2||call.arguments[0].type!=='Identifier'||call.arguments[0].name!=='name'||call.arguments[1].type!=='Identifier'||call.arguments[1].name!=='value')context.report({node,messageId:'invalid'});
      }
    }};
  },
};
