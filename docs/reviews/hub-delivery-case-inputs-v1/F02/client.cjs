exports.load=async base=>{const r=await fetch(base+'/order');const x=await r.json();if(typeof x.amount!=='number')throw Error('amount must be number');return x.amount.toFixed(2)};
