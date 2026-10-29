import React from 'react';
export default function createModule() {
  return {
    Component() {
      return React.createElement('div', null, 'bare-import');
    },
  };
}
