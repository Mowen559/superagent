const {dispatch}=require('./runtime.cjs');exports.handleTask=task=>dispatch(()=>task.value);
